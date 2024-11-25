curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data @- <<EOF
$(jq -n --arg text "$(cat loc_report.md)" '{
    "blocks": [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Weekly ethrex lines of code report"
            }
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
