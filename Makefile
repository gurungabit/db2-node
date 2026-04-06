.PHONY: all build test test-unit test-integration test-node db2-start db2-stop db2-status docs-build docs-serve clean

MKDOCS_IMAGE ?= squidfunk/mkdocs-material:9.7.6
DOCS_OUTPUT_DIR ?= .tmp-docs-public

all: build test

build:
	cargo build --workspace

build-release:
	cargo build --workspace --release

db2-start:
	@chmod +x tools/db2.sh
	@./tools/db2.sh start

db2-stop:
	@./tools/db2.sh stop

db2-status:
	@./tools/db2.sh status

db2-reset:
	@./tools/db2.sh reset

test-unit:
	cargo test --workspace --lib
	cargo test -p db2-proto

test-integration: db2-ensure
	DB2_TEST_HOST=localhost \
	DB2_TEST_PORT=50000 \
	DB2_TEST_DATABASE=testdb \
	DB2_TEST_USER=db2inst1 \
	DB2_TEST_PASSWORD=db2wire_test_pw \
	cargo test --workspace --test '*' -- --test-threads=4

test-node: db2-ensure
	cd crates/db2-napi && npm install && npm test

docs-build:
	docker run --rm -v $(CURDIR):/docs $(MKDOCS_IMAGE) build --clean --site-dir $(DOCS_OUTPUT_DIR)

docs-serve:
	docker run --rm -p 8000:8000 -v $(CURDIR):/docs $(MKDOCS_IMAGE) serve --dev-addr 0.0.0.0:8000

test: test-unit test-integration

db2-ensure:
	@./tools/db2.sh status > /dev/null 2>&1 || ./tools/db2.sh start

capture:
	@./tools/db2.sh capture

clean:
	cargo clean
	@./tools/db2.sh stop 2>/dev/null || true
