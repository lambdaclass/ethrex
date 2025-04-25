#!/bin/bash

set -e

## Deps

sudo apt update
sudo apt install -y tmux build-essential pkg-config libssl-dev libclang-dev libgmp3-dev jq autoconf m4 libncurses5-dev libgl1-mesa-dev libglu1-mesa-dev libssh-dev unixodbc-dev

### Rust
curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh -s -- -y

### ASDF
git -C ~/.asdf pull origin v0.14.1 --ff-only || git clone https://github.com/asdf-vm/asdf.git ~/.asdf --branch v0.14.1
bash -c '. "$HOME/.asdf/asdf.sh"'
bash -c '. "$HOME/.asdf/completions/asdf.bash"'

### Erlang
asdf plugin add erlang https://github.com/asdf-vm/asdf-erlang.git
asdf install erlang 27.1
asdf global erlang 27.1

### Elixir
asdf plugin add elixir https://github.com/asdf-vm/asdf-elixir.git
asdf install elixir 1.17.3-otp-27
asdf global elixir 1.17.3-otp-27

### NodeJS
asdf plugin add nodejs https://github.com/asdf-vm/asdf-nodejs.git
asdf install nodejs 20.17.0 # blockscout-backend
asdf install nodejs 22.11.0 # blockscout-frontend
asdf global nodejs 22.11.0  # making blockscout-frontend's version global because the blockscout-backend has a .tool-versions that overrides the global one.
export PATH=$PATH:$(npm get prefix -g)/bin

### Docker
curl -fsSL https://get.docker.com -o get-docker.sh
sudo sh get-docker.sh
sudo usermod -aG docker $USER

## Deploy

docker compose -f ethrex-explorer/docker-compose.yml up -d

git -C ethrex-explorer/blockscout-backend pull origin master --ff-only || git clone https://github.com/blockscout/blockscout ethrex-explorer/blockscout-backend

cd ethrex-explorer/blockscout-backend

#cd ./docker-compose
#docker compose -f microservices.yml up -d

#cd ..

export DATABASE_URL=postgresql://blockscout:blockscout@localhost:5432/blockscout
mix do deps.get, local.rebar --force, deps.compile
mix phx.gen.secret > secret_key
export SECRET_KEY_BASE=$(cat secret_key)
export ETHEREUM_JSONRPC_VARIANT=geth
export ETHEREUM_JSONRPC_HTTP_URL=http://ethrex-l2-validium-staging:1729

mix compile
mix do ecto.create, ecto.migrate
cd apps/block_scout_web/assets; npm install && node_modules/webpack/bin/webpack.js --mode production; cd -
cd apps/explorer && npm install; cd -
mix phx.digest
cd apps/block_scout_web; mix phx.gen.cert blockscout blockscout.local; cd -

export MICROSERVICE_SC_VERIFIER_ENABLED=true
export MICROSERVICE_SC_VERIFIER_URL=http://localhost:8082/
export MICROSERVICE_VISUALIZE_SOL2UML_ENABLED=true
export MICROSERVICE_VISUALIZE_SOL2UML_URL=http://localhost:8081/
export MICROSERVICE_SIG_PROVIDER_ENABLED=true
export MICROSERVICE_SIG_PROVIDER_URL=http://localhost:8083/

tmux new -d -s backend
tmux send-keys -t backend ". "$HOME/.asdf/asdf.sh" && mix phx.server" Enter

cd ~
git -C ethrex-explorer/blockscout-frontend pull origin v0.14.1 --ff-only || git clone https://github.com/blockscout/frontend ethrex-explorer/blockscout-frontend
cd ethrex-explorer/blockscout-frontend
echo 'NEXT_PUBLIC_API_HOST=localhost
NEXT_PUBLIC_API_PORT=3001
NEXT_PUBLIC_API_PROTOCOL=http
NEXT_PUBLIC_STATS_API_HOST=http://localhost:8080
NEXT_PUBLIC_VISUALIZE_API_HOST=http://localhost:8081
NEXT_PUBLIC_APP_HOST=localhost
NEXT_PUBLIC_APP_PORT=3000
NEXT_PUBLIC_APP_INSTANCE=localhost
NEXT_PUBLIC_APP_ENV=development
NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL='ws'
NEXT_PUBLIC_WALLET_CONNECT_PROJECT_ID=' > .env

yarn
tmux new -d -s frontend
tmux send-keys -t frontend ". "$HOME/.asdf/asdf.sh" && asdf reshim nodejs && source .env && yarn dev" Enter

