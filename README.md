# Spotijay

Warning: This is a hastily done hack-job of a project. Don't judge.

## Running it

Run the server with: `cargo run --bin server`  
Run the (non-functional) CLI client with: `cargo run --bin client`

The webclient is a bit more complicated to get going as it uses wasm-pack and microserver.  
Install `cargo-make`  
Build the webclient with: `cargo make watch`  
Run tests with: `cargo make test`  
Run microserver with: `cargo make serve`  