# image.raw is set to phony because the list of files that require rebuilding it is huge
.PHONY: run clean image.raw

IMAGE_NAME = ethrex-image_0.1.raw
NIXPKGS_URL = https://github.com/NixOS/nixpkgs/archive/3fcbdcfc707e0aa42c541b7743e05820472bdaec.tar.gz
NIX_BUILD_ARGS = --no-out-link -I nixpkgs=$(NIXPKGS_URL)

image.raw:
	$(eval IMAGE := $(shell nix-build image.nix ${NIX_BUILD_ARGS})/${IMAGE_NAME})
	cp $(IMAGE) image.raw
	chmod u+rw image.raw

run: image.raw
	$(eval RUN_QEMU := $(shell nix-build hypervisor.nix ${NIX_BUILD_ARGS})/bin/run-qemu)
	$(RUN_QEMU) image.raw

clean:
	rm image.raw
