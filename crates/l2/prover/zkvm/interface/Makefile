# This Makefile is intended to be invoked only by build.rs

pico_elf := ${OUT_DIR}/riscv32im-pico-zkvm-elf

$(pico_elf):
	cd pico; \
		cargo pico build --output-directory ${OUT_DIR}
