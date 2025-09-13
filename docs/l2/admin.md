# Admin API

This API exposes endpoints to manage the Sequencer.

## Base URL

By default the server is listening on `127.0.0.1:5555` but can be configured with `--admin-server.addr <address>` `--admin-server.port <port>`

## Endpoints

### L1 Committer

---

#### Start Committer immediately

**Description**

Starts the committer immediately (with a delay of 0).

**Endpoint**

```
GET /committer/start
```

**Example**

```
curl -X GET http://localhost:5555/committer/start
```

---

#### Start Committer (with delay)

**Description**

Starts the committer with a configurable delay.

**Endpoint**

```
GET /committer/start/{delay}
```

**Example**

```
curl -X GET http://localhost:5555/committer/start/60000
```

**Parameters**

|Name|Type|Description|
|----|----|-----------|
|delay|number|Delay in milliseconds before starting the committer.|

---

#### Stop Committer

**Description**

Stops the committer.

**Endpoint**

```
GET /committer/stop
```

**Example**

```
curl -X GET http://localhost:5555/committer/stop
```
