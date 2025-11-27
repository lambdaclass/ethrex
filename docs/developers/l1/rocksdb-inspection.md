# Database inspection

For debugging purposes, you might want to inspect the contents of the database.

You can use the [RocksDB tools](https://github.com/facebook/rocksdb/wiki/Administration-and-Data-Access-Tool) to query the current state of the snapshotting process.

You may need to build rocksdb from source, since distributions might package outdated versions not supporting newer formats.

The procedure for doing that, in Debian 12, is:
```
sudo apt install libgflags-dev libsnappy-dev zlib1g-dev libbz2-dev liblz4-dev libzstd-dev
git clone https://github.com/facebook/rocksdb.git
cd rocksdb
make all
```

You can then use the ldb tool to query or modify (ensure the node is offline!) the state.
