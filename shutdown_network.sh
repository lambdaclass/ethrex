#!/bin/bash

pids_path="data/pids.txt"
datadirs_path="data/datadirs.txt"
rpcs_path="data/rpcs.txt"

kill_processes() {
    if [ -f $pids_path ]; then
        while read pid; do
            if [ -n "$pid" ] && ps -p $pid > /dev/null; then
                kill $pid
                echo "Killed process $pid"
            fi
        done < $pids_path

        rm -f $pids_path
        echo "Removed pids.txt"
    else
        echo "No processes to kill.";
    fi
}

clean_datadirs() {
    datadir_root="/Users/ivanlitteri/Library/Application Support"

    if [ -s $datadirs_path ]; then
        while read datadir; do
            if [ -n "$datadir" ]; then
                datadir_path="$datadir_root/$datadir"
                rm -rf "$datadir_path"
                echo "Removed datadir $datadir_path"
            fi
        done < $datadirs_path
        
        rm -f $datadirs_path
        echo "Removed datadirs.txt"
    else 
        echo "No datadirs to remove.";
    fi
}

clean_rpcs() {
    if [ ! -s $rpcs_path ]; then
        echo "No RPC file to remove."
        exit 0
    fi

    rm -f $rpcs_path
    echo "Removed rpcs.txt"
}

kill_processes
clean_datadirs
clean_rpcs
