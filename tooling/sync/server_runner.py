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

    return parser.parse_args()


def send_slack_message(message: str):
    try:
        webhook_url = os.environ["SLACK_WEBHOOK_URL"]
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


def main():
    args = parse_args()
    variables = {}
    hostname = socket.gethostname()

    # Only include SNAP if flag is set
    if args.snap:
        variables["SNAP"] = "1"
    if args.healing:
        variables["HEALING"] = "1"
    if args.memory:
        variables["MEMORY"] = "1"
    variables["SERVER_SYNC_NETWORK"] = args.network
    variables["SERVER_SYNC_BRANCH"] = args.branch

    logs_file = args.logs_file
    command = ["make", "server-sync"]

    for key, value in variables.items():
        command.append(f"{key}={value}")

    payload = {"jsonrpc": "2.0", "method": "eth_syncing", "params": [], "id": 1}
    try:
        while True:
            start_time = time.time()
            subprocess.run(
                command + [f"LOGS_FILE={logs_file}_{start_time}.log"], check=True
            )
            while True:
                try:
                    elapsed = time.time() - start_time
                    if elapsed > args.timeout * 60:
                        print(
                            f"⚠️ Node did not sync within {args.timeout} minutes. Stopping."
                        )
                        send_slack_message(
                            f"⚠️ Node on {hostname} did not sync within {args.timeout} minutes. Stopping. Log File: {logs_file}_{start_time}.log"
                        )
                        with open("sync_logs.txt", "a") as f:
                            f.write(f"LOGS_FILE={logs_file}_{start_time}.log FAILED\n")
                        break
                    response = requests.post(RPC_URL, json=payload).json()
                    result = response.get("result")
                    if result is False:
                        print("✅ Node is fully synced!")
                        with open("sync_logs.txt", "a") as f:
                            f.write(f"LOGS_FILE={logs_file}_{start_time}.log SYNCED\n")
                        break
                    time.sleep(CHECK_INTERVAL)
                except Exception as e:
                    pass
    except subprocess.CalledProcessError as e:
        print(f"An error occurred while running the make command: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
