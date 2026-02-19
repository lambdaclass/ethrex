# Description

The term FlatKeyValue, also called "snapshots", refers to storing a direct key=>value mapping in the store.

This allows to replace several reads (depending on tree depth) with one.

# Design

Actors involved:
- Generate FlatKeyValue (GFKV): populates the flatkeyvalue table
- apply_updates (AU): updates the trie and updates the flatkeyvalue table
- block execution (X): reads the trie and the flatkeyvalue table

When the sync is complete (as soon as genesis is loaded in FullSync, or when SnapSync is done downloading tries) the GFKV process is signaled to start.

As GFKV completes the process, it saves to `last_written` how far it has gotten.

When AU needs to update the trie, it signals the GFKV process to stop so that it doesn't read a trie in the middle of writing. It only updates flatkeyvalues on the left of `last_written`.

After flushing the updates, AU signals GFKV to resume which reads the new state root and resumes from `last_written`.

When X needs to read values, it only uses the flatkeyvalues on the left of `last_written`, ie those it knows to be correct. Otherwise it goes through the trie.

# Debugging

You can inspect its progress by [inspecting the DB](./rocksdb-inspection.md).

In particular, you might want to run

```
./ldb --db=/home/$USER/.local/share/ethrex --column_family=misc_values --value_hex get last_written
```

to know how advanced the progress is.

A value of 0xff marks it as complete.
