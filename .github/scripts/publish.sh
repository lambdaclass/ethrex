curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data @- <<EOF
$(jq -n --arg text "$(cat results.md)" '{
    "blocks": [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Daily Hive Coverage report"
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
