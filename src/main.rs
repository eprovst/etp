/*
 * YE WHO ENTRE HERE, WELCOME TO THE SPAGHETTI MESS, NOT PROUD OF THIS
 * ONE BUT CAN'T BE BOTHERED...
 *                                           -- Evert
 */

use std::fs::File;
use std::io::{BufRead, BufReader, Error, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{exit, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() {
    if let Ok(apps) = load_apps() {
        if let Ok(listener) = TcpListener::bind("0.0.0.0:31337") {
            println!("[main] INFO: server started, listening on port 31337.");

            let mut id_counter = 0;
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let apps = apps.clone();
                id_counter = if id_counter >= 999 { 1 } else { id_counter + 1 };
                thread::spawn(move || {
                    handle_connection(id_counter, &mut stream, &apps);
                });
            }
        } else {
            println!("[main] ERROR: failed to bind listener to port 31337.");
            exit(1);
        }
    } else {
        println!("[main] ERROR: failed to load 'apps.txt'.");
        exit(1);
    }
}

fn handle_connection(id: u16, stream: &mut TcpStream, apps: &Vec<String>) {
    let mut req = Vec::<u8>::new();
    if let Ok(cstream) = stream.try_clone() {
        if let Ok(_) = BufReader::new(cstream).read_until(0, &mut req) {
            if req.len() > 0 {
                match req[0].into() {
                    'D' => {
                        if req.len() > 1 + 1 {
                            if let Ok(str) = String::from_utf8(req[1..req.len() - 1].to_vec()) {
                                let mut args = str.split_whitespace();
                                if let Some(app) = args.next() {
                                    run_app(
                                        id,
                                        stream,
                                        app.to_string(),
                                        args.map(|s| s.to_string()).collect(),
                                        apps,
                                    );
                                } else {
                                    println!("[{}] WARN: mangled arguments, dropping.", id);
                                }
                            } else {
                                println!("[{}] WARN: mangled app name, dropping.", id);
                            }
                        } else {
                            println!("[{}] WARN: too short request, dropping.", id);
                        }
                    }
                    'I' => info(id, stream),
                    t => {
                        println!("[{}] WARN: unknown request type '{}', dropping.", id, t);
                    }
                }
            } else {
                println!("[{}] WARN: empty request, dropping.", id);
            }
        } else {
            println!("[{}] WARN: failed to retrieve request, dropping.", id);
        }
    } else {
        println!("[{}] NEVER?: failed to get connection from thread.", id);
    }
    println!("[{}] INFO: closing connection.", id);
    _ = stream.flush();
    _ = stream.shutdown(Shutdown::Both);
}

fn load_apps() -> Result<Vec<String>, Error> {
    BufReader::new(File::open("apps.txt")?).lines().collect()
}

fn run_app(id: u16, stream: &mut TcpStream, app: String, args: Vec<String>, apps: &Vec<String>) {
    println!("[{}] INFO: starting job '{} {}'.", id, app, args.join(" "));
    // This line does all the heavy lifting when it comes to security
    if apps.contains(&app) {
        if let Ok(mut child) = Command::new(app)
            .env("PATH", ".")
            .args(args)
            .stdout(Stdio::piped())
            .spawn()
        {
            // Create a buffer which actively reads from child stdout
            // as child does not stop if the output buffer is not empty
            // Hack #1
            if let Some(mut stdout) = child.stdout.take() {
                let buffer = Arc::new(Mutex::new(Vec::new()));
                let buffer_in = Arc::clone(&buffer);

                let reader = thread::spawn(move || {
                    if let Ok(mut buffer) = buffer_in.lock() {
                        let _ = stdout.read_to_end(&mut buffer);
                    }
                });

                // Active waiting as event driven stuff is hard/impossible
                // Hack #2
                // Also, so many fail states...
                loop {
                    // Child process done?
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                println!("[{}] WARN: app returned error, dropping.", id);
                                // Clean up reader
                                let _ = reader.join();
                                return;
                            } else {
                                if let Ok(_) = reader.join() {
                                    if let Ok(outp) = buffer.lock() {
                                        _ = write!(stream, "{}\0", outp.len());
                                        _ = stream.write_all(outp.as_slice());
                                        println!("[{}] INFO: job completed.", id);
                                    } else {
                                        println!("[{}] NEVER?: buffer lock failed, dropping.", id);
                                    }
                                } else {
                                    println!("[{}] WARN: reading stdout failed, dropping.", id);
                                }
                                return;
                            }
                        }
                        Err(_) => {
                            println!("[{}] WARN: app execution failed, dropping.", id);
                            // Clean up reader
                            let _ = reader.join();
                            return;
                        }
                        Ok(None) => {
                            if reader.is_finished() {
                                if let Ok(None) = child.try_wait() {
                                    // Reader failed but child still busy, stdout will never empty
                                    println!("[{}] WARN: reading app output failed, dropping.", id);
                                    // Kill child
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    return;
                                }
                            }
                        }
                    }

                    // Is connection closed?
                    if let Err(_) = stream.write(&[b'\0']) {
                        println!("[{}] WARN: client dropped connection, dropping.", id);
                        // Kill child
                        let _ = child.kill();
                        let _ = child.wait();
                        // Clean up reader
                        let _ = reader.join();
                        return;
                    }

                    // Give the process some time
                    thread::sleep(Duration::from_millis(100));
                }
            } else {
                println!(
                    "[{}] WARN: failed to connect to app's stdout, dropping.",
                    id
                );
                // Kill child
                let _ = child.kill();
                let _ = child.wait();
            }
        } else {
            println!("[{}] WARN: app failed to start, dropping.", id);
        }
    } else {
        // App is not allowed
        println!("[{}] WARN: '{}' not in 'apps.txt', dropping.", id, app);
    }
}

fn info(id: u16, stream: &mut TcpStream) {
    if let Ok(n) = thread::available_parallelism() {
        _ = write!(stream, "N{}\0", n);
        println!("[{}] INFO: sent info.", id);
    } else {
        _ = write!(stream, "N1\0");
        println!("[{}] WARN: could not read number of threads, assuming one.", id);
    }
}
