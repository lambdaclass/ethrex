// JSON deserialization types for Ethereum TransactionTest fixtures.
package spec

// TransactionTestSuite is the top-level map: test name → TransactionTestCase.
type TransactionTestSuite map[string]*TransactionTestCase

// TransactionTestCase is a single transaction test case.
type TransactionTestCase struct {
	Result  map[string]*TxTestResult `json:"result"`  // fork → result
	TxBytes HexBytes                 `json:"txbytes"`
}

// TxTestResult holds the expected result for a specific fork.
// If Exception is set, the transaction is invalid for this fork.
// Otherwise Hash, IntrinsicGas, and Sender should match.
type TxTestResult struct {
	Hash         *string `json:"hash,omitempty"`
	IntrinsicGas *string `json:"intrinsicGas,omitempty"`
	Sender       *string `json:"sender,omitempty"`
	Exception    *string `json:"exception,omitempty"`
}
