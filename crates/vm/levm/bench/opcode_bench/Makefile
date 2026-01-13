SHELL := /bin/bash

OPCODE_BENCH := cargo run --bin opcode_bench --

RUNS ?= 10
ITERS ?= 100000
CSV ?= bench_results.csv
CONFIG ?= run_config.json

.PHONY: list run run-category run-all csv-all csv-category csv-config

list:
	$(OPCODE_BENCH) list

run:
	@if [ -z "$(FIXTURE)" ]; then echo "set FIXTURE=<name>"; exit 1; fi
	$(OPCODE_BENCH) $(FIXTURE) $(RUNS) $(ITERS)

run-category:
	@if [ -z "$(CATEGORY)" ]; then echo "set CATEGORY=<name>"; exit 1; fi
	$(OPCODE_BENCH) category:$(CATEGORY) $(RUNS) $(ITERS)

run-all:
	@fixtures=$$($(OPCODE_BENCH) list | awk '/^-/{print $$2}'); \
	for f in $$fixtures; do $(OPCODE_BENCH) $$f $(RUNS) $(ITERS); done

csv-all:
	@rm -f $(CSV)
	@fixtures=$$($(OPCODE_BENCH) list | awk '/^-/{print $$2}'); \
	for f in $$fixtures; do OPCODE_BENCH_CSV=$(CSV) $(OPCODE_BENCH) $$f $(RUNS) $(ITERS); done
	@echo "wrote $(CSV)"

csv-category:
	@if [ -z "$(CATEGORY)" ]; then echo "set CATEGORY=<name>"; exit 1; fi
	@rm -f $(CSV)
	@OPCODE_BENCH_CSV=$(CSV) $(OPCODE_BENCH) category:$(CATEGORY) $(RUNS) $(ITERS)
	@echo "wrote $(CSV)"

csv-config:
	@rm -f $(CSV)
	@python - <<'PY'
		import json
		import os
		import subprocess

		config_path = os.environ.get('CONFIG', 'run_config.json')
		with open(config_path, 'r', encoding='utf-8') as f:
		    cfg = json.load(f)

		defaults = cfg.get('defaults', {})
		fixtures = cfg.get('fixtures', {})

		cmd_prefix = ['cargo', 'run', '--bin', 'opcode_bench', '--']

		csv_path = os.environ.get('CSV', 'bench_results.csv')

		for name in sorted(fixtures.keys()):
		    params = fixtures[name]
		    runs = str(params.get('runs', defaults.get('runs', 10)))
		    iterations = str(params.get('iterations', defaults.get('iterations', 100000)))
		    env = os.environ.copy()
		    env['OPCODE_BENCH_CSV'] = csv_path
		    subprocess.run(cmd_prefix + [name, runs, iterations], check=True, env=env)

		print(f"wrote {csv_path}")
		PY
