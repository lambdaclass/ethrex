.PHONY: all deps clean prove

all: deps app.bin prove

deps:
	rustup target add riscv32i-unknown-none-elf
	rustup component add llvm-tools-preview
	cargo install cargo-binutils
	cargo +nightly install --git https://github.com/matter-labs/zksync-airbender.git cli -F gpu --root ~/.local

app.bin:
	cargo build --release
	cargo objcopy --release -- -O binary app.bin

prove:
	mkdir -p ./output/

	~/.local/bin/cli prove \
	--bin app.bin \
	--input-file input.txt  \
	--output-dir ./output/ \
	--until final-recursion \
	--gpu

clean:
	rm app.bin
	rm app.elf
	rm app.text

# cargo build --release # easier errors
# cargo objcopy --release -- -O binary app.bin
# cargo objcopy --release -- -R .text app.elf
# cargo objcopy --release -- -O binary --only-section=.text app.text
