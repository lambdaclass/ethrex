# L2 dev environment

## Usage

1. Download the docker compose 

curl -L https://raw.githubusercontent.com/lambdaclass/ethrex/main/tooling/l2/dev/docker-compose.yaml -o docker-compose.yaml

2. Start the containers

```shell
docker compose up
```

this will launch:

- on localhost:8083 blockscout explorer for L1
- on localhost:8082 blockscout explorer for L2
- on localhost:1729 l2 rpc
- on localhost:8545 l1 rpc
- on localhost:5173 ethrex L2 hub for withdrawals deposits and account abstraction

3. Stop the containers and delete the volumes

> [!NOTE]
> It is recommended to delete all the volumes because blockscout will keep the old state of the blockchain on its db
> but ethrex l2 dev mode starts a new chain on every restart. For this reason we use the `-v` flag

```shell
docker compose down -v
```
