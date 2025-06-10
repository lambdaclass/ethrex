.PHONY: loc loc-stats loc-detailed loc-compare-detailed

loc:
	cargo run

loc-stats:
	if [ "$(QUIET)" = "true" ]; then \
		cargo run --quiet -- --summary;\
	else \
		cargo run -- --summary;\
	fi

loc-detailed:
	cargo run -- --detailed

loc-compare-detailed:
	cargo run -- --compare-detailed
