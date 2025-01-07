curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data @- <<EOF
$(jq -n --arg text "$(diff.md)" '{
    "blocks": [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Default features vs LEVM passing Hive tests"
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
