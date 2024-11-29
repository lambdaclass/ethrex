curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data @- <<EOF
$(jq -n --arg text "$(cat loc_report_slack.txt)" '{
    "blocks": [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Lines of Code Report"
            }
        },
        {
            "type": "divider"
        },
        {
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": $text
            }             
        }
    ]
}')
EOF
