ARG baseimage=consensys/web3signer
ARG tag=develop

FROM $baseimage:$tag AS builder
COPY web3signer-key.yml ./keys/key_0.yaml

    ENTRYPOINT [ "/opt/web3signer/bin/web3signer", "--http-host-allowlist=*", "--key-store-path=./keys", "eth1", "--chain-id=1729" ]
