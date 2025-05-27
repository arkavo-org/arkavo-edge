Feature: Secure Banking Transactions
  As a banking customer
  I want my transactions to be secure
  So that my money is protected

  Background:
    Given a user account with balance $1000
    And the daily withdrawal limit is $500

  Scenario: Successful withdrawal within limits
    When I withdraw $200
    Then my balance should be $800
    And the transaction should be recorded

  Scenario: Withdrawal exceeding daily limit
    Given I have already withdrawn $400 today
    When I attempt to withdraw $200
    Then the transaction should be denied
    And I should see "Daily limit exceeded"

  @chaos @performance
  Scenario: Withdrawal under network issues
    Given network latency is 500ms
    And packet loss is 10%
    When I withdraw $100
    Then the transaction should complete within 2 seconds
    And no duplicate transactions should occur

  @security
  Scenario: Concurrent withdrawal attempts
    When I initiate 5 concurrent withdrawals of $100 each
    Then only one transaction should succeed
    And my balance should be $900
    And I should see 4 rejection messages

  @invariant
  Scenario: Balance never goes negative
    Given my account balance is $50
    When I attempt to withdraw $100
    Then the transaction should be denied
    And my balance should remain $50
    And the system should log the failed attempt