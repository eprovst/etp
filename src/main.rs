/*
 * YE WHO ENTRE HERE, WELCOME TO THE SPAGHETTI MESS, NOT PROUD OF THIS
 * ONE BUT CAN'T BE BOTHERED...
 *                                           -- Evert
 */

use std::fs::File;
use std::io::{BufRead, BufReader, Error, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::process::{exit, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() {
    // ETP auto discovery
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:31337") {
        println!("[main] INFO: ETP auto discovery started, listening on port udp/31337.");
        thread::spawn(move || loop {
            let mut buf = [0; 1];
            if let Ok((amt, src)) = socket.recv_from(&mut buf) {
                if amt == 1 && buf == ['P' as u8] {
                    if let Ok(1) = socket.send_to(&buf, src) {
                        println!("[ad] INFO: replied to auto discovery request from {src}.");
                    } else {
                        println!("[ad] WARN: failed to reply to {src}.");
                    }
                } else {
                    println!("[ad] WARN: invalid request, ignoring.");
                }
            } else {
                println!("[ad] WARN: failed to retrieve data, ignoring.");
            }
        });
    } else {
        println!("[main] ERROR: failed to bind UDP socket on port 31337.");
        exit(1);
    }

    // Main ETP server
    if let Ok(apps) = load_apps() {
        if let Ok(listener) = TcpListener::bind("0.0.0.0:31337") {
            println!("[main] INFO: ETP server started, listening on port tcp/31337.");

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
            println!("[main] ERROR: failed to bind TCP listener to port 31337.");
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
                                    println!("[{id}] WARN: mangled arguments, dropping.");
                                }
                            } else {
                                println!("[{id}] WARN: mangled app name, dropping.");
                            }
                        } else {
                            println!("[{id}] WARN: too short request, dropping.");
                        }
                    }
                    'I' => info(id, stream),
                    t => {
                        println!("[{id}] WARN: unknown request type '{t}', dropping.");
                    }
                }
            } else {
                println!("[{id}] WARN: empty request, dropping.");
            }
        } else {
            println!("[{id}] WARN: failed to retrieve request, dropping.");
        }
    } else {
        println!("[{id}] NEVER?: failed to get connection from thread.");
    }
    println!("[{id}] INFO: closing connection.");
    _ = stream.flush();
    _ = stream.shutdown(Shutdown::Both);
}

fn load_apps() -> Result<Vec<String>, Error> {
    BufReader::new(File::open("apps.txt")?).lines().collect()
}

fn run_app(id: u16, stream: &mut TcpStream, app: String, args: Vec<String>, apps: &Vec<String>) {
    println!("[{id}] INFO: starting job '{app} {}'.", args.join(" "));
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
                                println!("[{id}] WARN: app returned error, dropping.");
                                // Clean up reader
                                let _ = reader.join();
                                return;
                            } else {
                                if let Ok(_) = reader.join() {
                                    if let Ok(outp) = buffer.lock() {
                                        _ = write!(stream, "{}\0", outp.len());
                                        _ = stream.write_all(outp.as_slice());
                                        println!("[{id}] INFO: job completed.");
                                    } else {
                                        println!("[{id}] NEVER?: buffer lock failed, dropping.");
                                    }
                                } else {
                                    println!("[{id}] WARN: reading stdout failed, dropping.");
                                }
                                return;
                            }
                        }
                        Err(_) => {
                            println!("[{id}] WARN: app execution failed, dropping.");
                            // Clean up reader
                            let _ = reader.join();
                            return;
                        }
                        Ok(None) => {
                            if reader.is_finished() {
                                if let Ok(None) = child.try_wait() {
                                    // Reader failed but child still busy, stdout will never empty
                                    println!("[{id}] WARN: reading app output failed, dropping.");
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
                        println!("[{id}] WARN: client dropped connection, dropping.");
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
                println!("[{id}] WARN: failed to connect to app's stdout, dropping.");
                // Kill child
                let _ = child.kill();
                let _ = child.wait();
            }
        } else {
            println!("[{id}] WARN: app failed to start, dropping.");
        }
    } else {
        // App is not allowed
        println!("[{id}] WARN: '{app}' not in 'apps.txt', dropping.");
    }
}

fn info(id: u16, stream: &mut TcpStream) {
    if let Ok(n) = thread::available_parallelism() {
        _ = write!(stream, "N{n}\0");
        println!("[{id}] INFO: sent info.");
    } else {
        _ = write!(stream, "N1\0");
        println!("[{id}] WARN: could not read number of threads, assuming one.");
    }
}
