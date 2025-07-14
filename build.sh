#RUSTFLAGS="-C target-cpu=native -C link-arg=-static -C link-arg=-nostartfiles" cargo build --release
RUSTFLAGS="-C target-cpu=native link-arg=-nostartfiles" cargo build