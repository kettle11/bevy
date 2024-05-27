set -e
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory' \
     cargo build --example wasm_threading --target wasm32-unknown-unknown -Z build-std=std,panic_abort --release --features "bevy_dedicated_web_worker"
 wasm-bindgen --out-name wasm_example \
   --out-dir examples/wasm/target \
   --target web target/wasm32-unknown-unknown/release/examples/wasm_threading.wasm
 devserver --header Cross-Origin-Opener-Policy='same-origin' --header Cross-Origin-Embedder-Policy='require-corp' --path examples/wasm