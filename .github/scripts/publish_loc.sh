curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data "$(cat loc_report_slack.txt)"
