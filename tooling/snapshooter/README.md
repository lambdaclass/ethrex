# Snapshooter

## 1. Generate DB

### 1.1 From a new snapsync

#### 1.1.1 Clone ethrex in you machine

```shell
git clone git@github.com:lambdaclass/ethrex.git
```

#### 1.1.2 Checkout to `ansible-dev-deps`

```shell
cd ethrex

git checkout ansible-dev-deps
```

#### 1.1.3 Install dependencies remotely to a server

```
make -C ansible inventory L1_IP=<server_name> && make -C ansible ethrex-dev-deps BRANCH=<branch>
```

#### 1.1.3 In the remote server create a `.env` file in `ethrex/tooling/sync`

```shell
SLACK_WEBHOOK_URL_FAILED=
SLACK_WEBHOOK_URL_SUCCESS=
ARGS="--snap --network <network> --branch <branch> --timeout <timeout_in_minutes>"
```

#### 1.1.4 Run the service

```shell
sudo systemctl start snap_sync_runner
```

#### 1.1.5 Stop the process after snap sync finishes

```shell
sudo systemctl stop snap_sync_runner
```

### 1.2 From downloading the .gzip

```shell
scp admin@<server_name>:/path/to/db.bz2 /local/destination/path/to/db.bz2
```

## 2. Find the pivot block

## 3. Build the snapshooter

```shell
cargo b -r -p snapshooter
```

## 4. Run the binary

### 4.1 Run without any profiling tools (recommended if you are only interested in seeing time changes)

```shell
./target/release/snapshooter
```

### 4.2 Run with Samply

#### 4.2.1 Install Samply (skip if you already have it)

```shell
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/mstange/samply/releases/download/samply-v0.13.1/samply-installer.sh | sh
```

#### 4.2.2 Grant permissions to Samply

```shell
sudo sysctl kernel.perf_event_paranoid=1
```

#### 4.2.3 Restart SSH Session

```shell
exit

ssh -L 3000:localhost:3000 admin@<server_name>
```

#### 4.2.4 Run Samply

```shell
samply record ./target/release/snapshooter
```

### 4.3 Run with Flamegraph

#### 4.3.1 Install Cargo Flamegraph

```shell
cargo install flamegraph
```

#### 4.3.2 Grant Permissions to Flamegraph

```shell
sudo chown -R admin:admin /home/admin/.local/share/ethrex/
```

Then exit the SSH session and re-join.

#### 4.3.3 Run Flamegraph

```shell
CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph -p snapshooter
```

#### 4.3.4 Download `flamegraph.svg`

```shell
scp admin@<server_name>:/path/to/flamegraph.svg /local/path/to/flamegraph.svg
```
