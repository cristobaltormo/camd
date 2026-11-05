.PHONY: install test lint format web firmware image screenshots brand run

install:
	cd server && pip install -e '.[dev]'

test:
	cd server && pytest -q

lint:
	cd server && ruff check . && ruff format --check .

format:
	cd server && ruff check --fix . && ruff format .

web:
	cd server && node --test webtests/

firmware:
	cd firmware && pio run

image:
	docker build -t camd .

screenshots:
	tools/run-screenshots.sh

brand:
	tools/render-brand.sh

run:
	cd server && python -m camd
