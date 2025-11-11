.PHONY: all deps notebook

all: notebook

.venv:
	uv venv
	uv pip install -r requirements.txt

deps: .venv

notebook: deps
	uv run jupyter notebook log_analysis.ipynb
