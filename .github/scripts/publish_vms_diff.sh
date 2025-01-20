curl -X POST $1 \
-H 'Content-Type: application/json; charset=utf-8' \
--data @- <<EOF
$(jq -n --arg text "$(cat diff.md)" '{
    "blocks": [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Hive tests revm vs levm diff"
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
