import subprocess
import sys
import argparse
import requests
import time
import os
import json
import socket


RPC_URL = "http://localhost:8545"
CHECK_INTERVAL = 5  # seconds


def parse_args():
    parser = argparse.ArgumentParser(
        description="Run a Makefile with optional variables."
    )
    parser.add_argument("--snap", action="store_true", help="Whether snap is activated")
    parser.add_argument(
        "--healing", action="store_true", help="Whether healing is activated"
    )
    parser.add_argument(
        "--memory", action="store_true", help="Whether memory is activated"
    )
    parser.add_argument(
        "--network", type=str, default="hoodi", help="Network variable (default: hoodi)"
    )
    parser.add_argument(
        "--branch",
        type=str,
        default="snap_sync",
        help="Branch variable (default: snap_sync)",
    )
    parser.add_argument(
        "--logs_file",
        type=str,
        default="output",
        help="Logs file name (default: output)",
    )
    parser.add_argument(
        "--timeout", type=int, default=60, help="Timeout in minutes (default: 60)"
    )
    parser.add_argument(
        "--no-monitor",
        action="store_true",
        help="Whether we should restart after success/failure",
    )
    parser.add_argument(
        "--block_wait_time",
        type=int,
        default=60,
        help="Time to wait until new block in seconds (default: 60)",
    )
    parser.add_argument(
        "--debug-assert", action="store_true", help="Whether it should be compiled with debug assertions"
    )

    return parser.parse_args()


def send_slack_message_failed(message: str):
    try:
        webhook_url = os.environ["SLACK_WEBHOOK_URL_FAILED"]
        message = {"text": message}
        response = requests.post(
            webhook_url,
            data=json.dumps(message),
            headers={"Content-Type": "application/json"},
        )

        if response.status_code != 200:
            print(f"Error sending Slack message")

    except Exception as e:
        print(f"Error sending Slack message: {e}", file=sys.stderr)
        return


def send_slack_message_success(message: str):
    try:
        webhook_url = os.environ["SLACK_WEBHOOK_URL_SUCCESS"]
        message = {"text": message}
        response = requests.post(
            webhook_url,
            data=json.dumps(message),
            headers={"Content-Type": "application/json"},
        )

        if response.status_code != 200:
            print(f"Error sending Slack message")

    except Exception as e:
        print(f"Error sending Slack message: {e}", file=sys.stderr)
        return


def get_variables(args):
    variables = {}

    # Only include SNAP if flag is set
    if args.snap:
        variables["SNAP"] = "1"
    if args.healing:
        variables["HEALING"] = "1"
    if args.memory:
        variables["MEMORY"] = "1"
    if args.debug_assert:
        variables["DEBUG_ASSERT"] = "1"
    variables["SERVER_SYNC_NETWORK"] = args.network
    variables["SERVER_SYNC_BRANCH"] = args.branch

    return variables


def block_production_loop(
    hostname, args, logs_file, elapsed, start_time, block_production_payload
):
    current_block_number = 0
    block_start_time = time.time()
    while True:
        block_elapsed = time.time() - block_start_time
        if block_elapsed > 30 * 60:  # 30 minutes
            print("✅ Node is fully synced!")
            send_slack_message_success(
                f"✅ Node on {hostname} is fully synced after {elapsed / 60:.2f} minutes and correctly generated blocks for 30 minutes! Network: {args.network} Log File: {logs_file}_{start_time}.log"
            )
            with open("sync_logs.txt", "a") as f:
                f.write(f"LOGS_FILE={logs_file}_{start_time}.log SYNCED\n")
            return True
        try:
            response = requests.post(RPC_URL, json=block_production_payload).json()
            result = response.get("result")
            if int(result, 0) > current_block_number:
                current_block_number = int(result, 0)
            else:
                print(f"⚠️ Node did not generated a new block. Stopping.")
                send_slack_message_failed(
                    f"⚠️ Node on {hostname} stopped generating new blocks after sync. Network: {args.network}. Stopping. Log File: {logs_file}_{start_time}.log"
                )
                with open("sync_logs.txt", "a") as f:
                    f.write(f"LOGS_FILE={logs_file}_{start_time}.log FAILED\n")
                return False
        except Exception as e:
            print(f"⚠️ Node did stopped. Stopping.")
            print("Error:", e)
            send_slack_message_failed(
                f"⚠️ Node on {hostname} stopped. Network: {args.network}. Log File: {logs_file}_{start_time}.log"
            )
            with open("sync_logs.txt", "a") as f:
                f.write(f"LOGS_FILE={logs_file}_{start_time}.log FAILED\n")
            return False
        time.sleep(args.block_wait_time)


def verification_loop(
    logs_file, args, hostname, payload, block_production_payload, start_time
):
    while True:
        try:
            elapsed = time.time() - start_time
            if elapsed > args.timeout * 60:
                print(f"⚠️ Node did not sync within {args.timeout} minutes. Stopping.")
                send_slack_message_failed(
                    f"⚠️ Node on {hostname} did not sync within {args.timeout} minutes. Network: {args.network}. Stopping. Log File: {logs_file}_{start_time}.log"
                )
                with open("sync_logs.txt", "a") as f:
                    f.write(f"LOGS_FILE={logs_file}_{start_time}.log FAILED\n")
                return False
            response = requests.post(RPC_URL, json=payload).json()
            result = response.get("result")
            if result is False:
                success = block_production_loop(
                    hostname,
                    args,
                    logs_file,
                    elapsed,
                    start_time,
                    block_production_payload,
                )
                return success
            time.sleep(CHECK_INTERVAL)
        except Exception as e:
            pass


def execution_loop(
    command, logs_file, args, hostname, payload, block_production_payload
):
    while True:
        start_time = time.time()
        subprocess.run(
            command + [f"LOGS_FILE={logs_file}_{start_time}.log"], check=True
        )
        if args.no_monitor:
            print("No monitor flag set, exiting.")
            break
        success = verification_loop(
            logs_file, args, hostname, payload, block_production_payload, start_time
        )
        if not success:
            break


def main():
    args = parse_args()
    hostname = socket.gethostname()

    variables = get_variables(args)

    logs_file = args.logs_file
    command = ["make", "server-sync"]

    for key, value in variables.items():
        command.append(f"{key}={value}")

    payload = {"jsonrpc": "2.0", "method": "eth_syncing", "params": [], "id": 1}
    block_production_payload = {
        "jsonrpc": "2.0",
        "method": "eth_blockNumber",
        "params": [],
        "id": 1,
    }
    try:
        execution_loop(
            command, logs_file, args, hostname, payload, block_production_payload
        )
    except subprocess.CalledProcessError as e:
        print(f"An error occurred while running the make command: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
