.PHONY: release dist-serve doctor clean-release

release: ## Build release binary + package tarball
	scripts/package-release.sh

dist-serve: ## Serve dist/ over LAN for testing
	cd dist && python3 -m http.server 8000

doctor: ## Run lmml doctor against the local build
	cargo run --release -p lmml-tui -- doctor

clean-release: ## Remove target/release and dist/
	rm -rf target/release dist/
