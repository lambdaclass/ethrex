curl -XPOST -H "Content-type: application/json" -d '{
  "blocks": [
    {
      "type": "header",
      "text": {
        "type": "plain_text",
        "text": "🔥 Daily Flamegraph Report"
      }
    },
    {
      "type": "divider"
    },
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "Flamegraphs are available at *<https://lambdaclass.github.io/ethrex/|https://lambdaclass.github.io/ethrex/>*\n
        • *<https://lambdaclass.github.io/ethrex/flamegraph_ethrex.svg/flamegraph_ethrex.svg|Ethrex>*\n
        • *<https://lambdaclass.github.io/ethrex/flamegraph_reth.svg/flamegraph_reth.svg|Reth>*\n"
      }
    },
  ],
  "unfurl_links": true,
  "unfurl_media": true
}' "$1"
