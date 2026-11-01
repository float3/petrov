//! Multiplayer Petrov Day, on its own server. See the library for the
//! game itself.

use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let addr = std::env::var("PETROV_ADDR").unwrap_or_else(|_| "127.0.0.1:8095".into());
    let state =
        PathBuf::from(std::env::var("PETROV_STATE").unwrap_or_else(|_| "games.json".into()));
    let groups = state.with_file_name("groups.json");
    let brand = std::env::var("PETROV_BRAND").unwrap_or_else(|_| "petrov".into());
    let petrov = petrov::load(state, groups, &brand)
        .unwrap_or_else(|| panic!("unknown PETROV_BRAND {brand}"));
    rooms::serve(&addr, petrov::router(petrov)).await;
}
