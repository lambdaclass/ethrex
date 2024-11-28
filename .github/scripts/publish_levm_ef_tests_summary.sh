curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data @- <<EOF
$(jq -n --arg text "$(cat levm_ef_tests_summary_slack.txt)" '{
    "blocks": [
        {
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Daily LEVM EF Tests Run Report"
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
