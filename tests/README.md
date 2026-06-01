# Balance Characterization Tests

These tests define the strict business logic invariants of the bank statement modification engine. They serve as a hard gate.

**CRITICAL:** These tests must continue to pass through every subsequent ticket. No L2 work or any future ticket is permitted to alter the tested invariants (e.g. rejecting both Debit and Credit on the same line, rejecting negative balances, etc.).
