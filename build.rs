//! Compiles the page's TypeScript into the binary. Petrov Day has a look
//! of its own, so it takes the script and not the shared stylesheet.

fn main() {
    rooms_client::script("client");
}
