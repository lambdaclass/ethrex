.PHONY: run clean

IMAGE_NAME = ethrex-image_0.1.raw
NIXPKGS_URL = https://github.com/NixOS/nixpkgs/archive/3fcbdcfc707e0aa42c541b7743e05820472bdaec.tar.gz
NIX_BUILD_ARGS = -I nixpkgs=$(NIXPKGS_URL)

image.raw:
	$(eval IMAGE := $(shell nix-build image.nix --no-out-link)/${IMAGE_NAME})
	cp $(IMAGE) image.raw
	chmod u+rw image.raw

run: image.raw
	$(eval RUN_QEMU := $(shell nix-build hypervisor.nix --no-out-link)/bin/run-qemu)
	rm result
	$(RUN_QEMU) image.raw

clean:
	rm image.raw
