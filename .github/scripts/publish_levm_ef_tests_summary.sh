curl -X POST $url \
-H 'Content-Type: application/json; charset=utf-8' \
--data "$(cat cmd/ef_tests/levm/levm_ef_tests_summary_slack.txt)"
