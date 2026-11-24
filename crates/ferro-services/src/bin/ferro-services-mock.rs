//! `ferro-services-mock`: the in-memory services server
//! (`ferro_services::mock`) for local development. Listens on 127.0.0.1
//! only; payments are fake.
//!
//! ```text
//! cargo run -p ferro-services --bin ferro-services-mock -- [--port 8787]
//! ```

use ferro_services::mock::{MockOptions, MockServer, ACTIVATION_CODE};

fn main() {
    let mut opts = MockOptions::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                match args.get(i + 1).and_then(|p| p.parse().ok()) {
                    Some(p) => opts.port = p,
                    None => {
                        eprintln!("--port needs a number");
                        std::process::exit(2);
                    }
                }
                i += 1;
            }
            "-h" | "--help" => {
                println!(
                    "ferro-services-mock [--port N]: a fake Ferro services server on 127.0.0.1"
                );
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let server = match MockServer::start(opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot start: {e}");
            std::process::exit(1);
        }
    };
    println!("Ferro mock services on {}", server.url());
    println!("Start Ferro against it with:");
    println!();
    println!(
        "  FERRO_SERVICES_URL={} FERRO_DEV_LICENSE_KEY={} ferro",
        server.url(),
        server.public_key_b64()
    );
    println!();
    println!("Sign-in approves itself after a few polls; activation code: {ACTIVATION_CODE}.");
    println!("Buying opens a fake payment page that marks the order paid. Ctrl+C to stop.");
    loop {
        std::thread::park();
    }
}
