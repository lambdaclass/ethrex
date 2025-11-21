# L2 dev environment

## Usage

1. Download the docker compose 

curl -L https://raw.githubusercontent.com/lambdaclass/ethrex/main/tooling/l2/dev/docker-compose.yaml -o docker-compose.yaml

2. Start the containers

```shell
docker compose up
```

3. Stop the containers and delete the volumes

> [!NOTE]
> It is recommended to delete all the volumes because blockscout will keep the old state of the blockchain on its db
> but ethrex l2 dev mode starts a new chain on every restart. For this reason we use the `-v` flag

```shell
docker compose down -v
```
