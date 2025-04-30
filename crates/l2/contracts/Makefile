.PHONY: deps deploy

deps:
	forge install foundry-rs/forge-std --no-git --no-commit 2>/dev/null || exit 0
	forge install OpenZeppelin/openzeppelin-foundry-upgrades --no-git --no-commit 2>/dev/null || exit 0
	forge install OpenZeppelin/openzeppelin-contracts-upgradeable --no-git --no-commit 2>/dev/null || exit 0

deploy:
	forge script scripts/deploy.s.sol \
	    --rpc-url localhost:8545 \
		--broadcast \
		--private-key ${PRIVATE_KEY} \
		-s 'run(address,bool)' 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 false
