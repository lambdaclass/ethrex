curl -X POST $1 \
-H 'Content-Type: application/json; charset=utf-8' \
--data "$(cat tooling/loc/loc_report_slack.txt)"
