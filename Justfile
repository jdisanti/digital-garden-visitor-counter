export RUSTFLAGS := "-Dwarnings"
export RUST_BACKTRACE := "1"

test:
    @echo "Checking code style..."
    cargo fmt --check
    @echo "Running tests..."
    cargo test
    @echo "Running clippy lints..."
    cargo clippy
    @echo "Compiling infrastructure..."
    cd infrastructure; npm install && npm run build
    @echo "SUCCESS!"

synth:
    @echo "Building with cargo-lambda..."
    cargo lambda build --output-format Zip --lambda-dir infrastructure/build --arm64 --release
    @echo "Synthesizing CDK infrastructure..."
    cd infrastructure; npm install && npm run build && npx cdk synth
    @echo "SUCCESS!"

deploy allowed-names='default,repo-readme' min-width='5': synth
    @echo "Deploying CDK infrastructure..."
    cd infrastructure; \
        npx cdk bootstrap && \
        npx cdk deploy digital-garden-visitor-counter \
            --parameters "allowedNames={{allowed-names}}" \
            --parameters "minWidth={{min-width}}"
    @echo "SUCCESS!"