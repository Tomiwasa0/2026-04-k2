# K2 audit details
- Total Prize Pool: $135,000 in USDC 
    - HM awards: up to $120,000 in USDC
        - If no valid Highs or Mediums are found, the HM pool is $0 
    - QA awards: $5,000 in USDC 
    - Judge awards: $9,500 in USDC 
    - Scout awards: $500 USDC
- [Read our guidelines for more details](https://docs.code4rena.com/competitions)
- Starts April 10, 2026 20:00 UTC 
- Ends May 20, 2026 20:00 UTC

### ❗ Important notes for wardens
1. Since this audit includes live/deployed code, **all submissions will be treated as sensitive**:
    - Wardens are encouraged to submit High-risk submissions affecting live code promptly, to ensure timely disclosure of such vulnerabilities to the sponsor and guarantee payout in the case where a sponsor patches a live critical during the audit.
    - Submissions will be hidden from all wardens (SR and non-SR alike) by default, to ensure that no sensitive issues are erroneously shared.
    - If the submissions include findings affecting live code, there will be no post-judging QA phase. This ensures that awards can be distributed in a timely fashion, without compromising the security of the project. (Senior members of C4 staff will review the judges’ decisions per usual.)
    - By default, submissions will not be made public until the report is published.
    - Exception: if the sponsor indicates that no submissions affect live code, then we’ll make submissions visible to all authenticated wardens, and open PJQA to SR wardens per the usual C4 process.
    - [The "live criticals" exception](https://docs.code4rena.com/awarding#the-live-criticals-exception) therefore applies.
1. A coded, runnable PoC is required for all High/Medium submissions to this audit. 
    - This repo includes a basic template to run the test suite.
    - PoCs must use the test suite provided in this repo.
    - Your submission will be marked as Insufficient if the POC is not runnable and working with the provided test suite.
    - Exception: PoC is optional (though recommended) for wardens with signal ≥ 0.4.
1. Judging phase risk adjustments (upgrades/downgrades):
    - High- or Medium-risk submissions downgraded by the judge to Low-risk (QA) will be ineligible for awards.
    - Upgrading a Low-risk finding from a QA report to a Medium- or High-risk finding is not supported.
    - As such, wardens are encouraged to select the appropriate risk level carefully during the submission phase.

## Publicly known issues

_Anything included in this section is considered a publicly known issue and is therefore ineligible for awards._

Self-liquidation - Users can liquidate their own positions. This follows Aave V3 precedent and provides a mechanism for users to efficiently unwind positions.

**Flash liquidation memory budget:** The two-step flash liquidation (`prepare_liquidation` + `execute_liquidation`) with a swap handler exceeds Soroban's 42 MB memory limit at 2+ reserves. Regular `liquidation_call` works up to ~6-7 reserves per user bitmap. This is a known Soroban VM constraint, not a protocol bug.

**Dex Integration:** Depth and liquidity of dex pools that may impact liquidation is a known issue and is considered risk-parameter related and OOS.

✅ SCOUTS: Please format the response above 👆 so its not a wall of text and its readable.

# Overview

[ ⭐️ SPONSORS: add info here ]

## Links

- **Previous audits:**  Halborn Security September 2025. WatchPug 4 audit rounds - October 2025 – March 2026
  - ✅ SCOUTS: If there are multiple report links, please format them in a list.
- **Documentation:** https://github.com/Shapeshifter-Technologies/k2-contracts/tree/main/docs
- **Website:** https://www.k2lend.com
- **X/Twitter:** https://x.com/K2_Lend
  
---

# Scope

[ ✅ SCOUTS: add scoping and technical details here ]

### Files in scope
- ✅ This should be completed using the `metrics.md` file
- ✅ Last row of the table should be Total: SLOC
- ✅ SCOUTS: Have the sponsor review and and confirm in text the details in the section titled "Scoping Q amp; A"

*For sponsors that don't use the scoping tool: list all files in scope in the table below (along with hyperlinks) -- and feel free to add notes to emphasize areas of focus.*

| Contract | SLOC | Purpose | Libraries used |  
| ----------- | ----------- | ----------- | ----------- |
| [contracts/folder/sample.sol](https://github.com/code-423n4/repo-name/blob/contracts/folder/sample.sol) | 123 | This contract does XYZ | [`@openzeppelin/*`](https://openzeppelin.com/contracts/) |

### Files out of scope
✅ SCOUTS: List files/directories out of scope

# Additional context

## Areas of concern (where to focus for bugs)
-

✅ SCOUTS: Please format the response above 👆 so its not a wall of text and its readable.

## Main invariants

**Solvency & Accounting**

- **Conservation of value:** For every reserve, `aToken_total_supply_scaled * liquidity_index` must not exceed `underlying_token_balance + total_debt_scaled * borrow_index` (modulo accrued protocol fees)
- **No phantom value creation:** aToken minting/burning must exactly correspond to underlying token deposits/withdrawals. Debt token minting/burning must exactly correspond to borrows/repayments.
- **Supply-debt consistency:** `sum(user_scaled_balances) == total_supply_scaled` for both aToken and debtToken contracts at all times

**Health Factor & Collateralization**

- **Borrowing collateralization:** After any borrow or withdraw, the user's health factor must be ≥ 1.0 WAD (1e18), or the transaction reverts with `HealthFactorTooLow` (Error #8)
- **Liquidation improvement:** After any liquidation, the borrower's health factor must improve (or remain within `LIQUIDATION_HF_TOLERANCE_BPS` = 1 bp tolerance for rounding noise)
- **Liquidation eligibility:** Only positions with health factor < `HEALTH_FACTOR_LIQUIDATION_THRESHOLD` (1.0 WAD) can be liquidated

**Index Monotonicity**

- **Liquidity index is non-decreasing:** `liquidity_index(t+1) >= liquidity_index(t)` — interest always accrues forward
- **Borrow index is non-decreasing:** `variable_borrow_index(t+1) >= variable_borrow_index(t)` — no interest regression
- **Indices never drop below RAY:** Both indices start at `RAY` (1e27) and can only increase

**Authorization**

- **Privileged operations require authorization:** All admin, configuration, and treasury operations require `require_auth()` from the appropriate role
- **Emergency admin asymmetry:** Emergency admin can pause but CANNOT unpause — only pool admin can unpause
- **User operations require user auth:** Supply, borrow, repay, withdraw all require `user.require_auth()` (except repay-on-behalf, which requires `repayer.require_auth()`)

**Flash Loans**

- **Atomicity:** Flash loaned assets must be repaid with premium within the same transaction, or the transaction reverts with `FlashLoanNotRepaid`
- **Premium collection:** Flash loan premium is always collected and rounds UP (`percent_mul_up`) — the protocol never receives less than the expected fee

**Protocol Safety**

- **Pause halts all state-changing operations:** When paused, all user operations (supply, borrow, repay, withdraw, liquidation, flash loan, swap) revert with `AssetPaused`. Read-only queries remain functional.
- **Reserve capacity limits:** Supply and borrow caps are enforced — no reserve can exceed its configured cap
- **Bitmap bounds:** User configuration bitmap supports at most 64 reserves (2 bits per reserve). `reserve_index >= 64` is rejected

✅ SCOUTS: Please format the response above 👆 so its not a wall of text and its readable.

## All trusted roles in the protocol

**Pool Admin**

- **Scope:** Full protocol configuration authority
- **Key operations:**
    - Initialize and configure reserves (via pool-configurator)
    - Update risk parameters: LTV, liquidation threshold, liquidation bonus, reserve factor
    - Set supply/borrow caps, min remaining debt, debt ceiling
    - Set flash loan premium (max 100 bps)
    - Set/change treasury, incentives, DEX router/factory addresses
    - Set swap handler whitelist, reserve whitelists/blacklists, liquidation whitelists/blacklists
    - Unpause protocol (emergency admin cannot)
    - Propose new pool admin or emergency admin (two-step transfer)
    - Upgrade contract WASM
    - Flush oracle config cache
- **Current holder:** Deployer key (target migration to Volta 2-of-5 multisig)

**Emergency Admin**

- **Scope:** Rapid incident response — pause only
- **Key operations:**
    - `pause()` — halt all user operations
    - **Cannot** unpause, configure reserves, or perform any other admin action
- **Rationale:** Separation ensures a compromised emergency key cannot unilaterally resume operations after an attack
- **Current holder:** Same as pool admin (target: separate key in multisig)

**Oracle Admin**

- **Scope:** Price feed configuration on the price-oracle contract
- **Key operations:**
    - Add/remove assets from oracle
    - Set manual price override (with expiry timestamp)
    - Set custom oracle, fallback oracle, batch oracle per asset
    - Set price cache TTL
    - Configure/reset circuit breaker
    - Pause/unpause oracle
- **Current holder:** Pool admin (same key)

**Pool Configurator Admin**

- **Scope:** Reserve deployment and parameter management
- **Key operations:**
    - Deploy and initialize new reserves (deploys aToken + debtToken contracts)
    - Configure collateral parameters
    - Update interest rate strategy per reserve
    - Drop reserves
- **Current holder:** Pool admin (same key)

**Emission Manager (Incentives)**

- **Scope:** Reward distribution configuration
- **Key operations:**
    - Configure per-asset reward emissions
    - Set emission rate (tokens/second)
    - Set distribution end timestamp
    - Fund reward pool with tokens
    - Pause/unpause incentives
- **Current holder:** Pool admin (same key)

**Treasury Admin**

- **Scope:** Protocol fee management
- **Key operations:**
    - Withdraw accumulated protocol fees
    - Sync token balances
- **Current holder:** Pool admin (same key)

**Interest Rate Strategy Admin**

- **Scope:** Rate curve parameters
- **Key operations:**
    - Set base variable borrow rate
    - Set variable rate slope 1 and slope 2
- **Current holder:** Pool admin (same key)

**Upgrade Admin (per contract, via shared upgradeable module)**

- **Scope:** WASM upgrade authority for each contract
- **Key operations:**
    - `upgrade(new_wasm_hash)` — replace contract bytecode
    - Two-step transfer: `propose_admin` → `accept_admin`
- **Current holder:** Pool admin (same key for all contracts)

✅ SCOUTS: Please format the response above 👆 using the template below👇

| Role                                | Description                       |
| --------------------------------------- | ---------------------------- |
| Owner                          | Has superpowers                |
| Administrator                             | Can change fees                       |

✅ SCOUTS: Please format the response above 👆 so its not a wall of text and its readable.

## Running tests

```
# Unit tests
cargo test --package k2-unit-tests

# Integration tests (requires release build for WASM-backed contract registration)
stellar contract build --release
cargo test --package k2-integration-tests --release

# Prerequisites
rustup install nightly
cargo install --locked cargo-fuzz
./deployment/build.sh  # WASM files required

# Run any fuzzer (macOS requires thread sanitizer flags)
cd contracts/kinetic-router/fuzz

RUSTFLAGS="-Cunsafe-allow-abi-mismatch=sanitizer" \
  cargo +nightly fuzz run --sanitizer=thread fuzz_target_1 -- -max_total_time=600

RUSTFLAGS="-Cunsafe-allow-abi-mismatch=sanitizer" \
  cargo +nightly fuzz run --sanitizer=thread fuzz_lending_operations -- -max_total_time=600

# List all targets
cargo +nightly fuzz list
```

✅ SCOUTS: Please format the response above 👆 using the template below👇

```bash
git clone https://github.com/code-423n4/2023-08-arbitrum
git submodule update --init --recursive
cd governance
foundryup
make install
make build
make sc-election-test
```
To run code coverage
```bash
make coverage
```

✅ SCOUTS: Add a screenshot of your terminal showing the test coverage

## Miscellaneous
Employees of K2 and employees' family members are ineligible to participate in this audit.

Code4rena's rules cannot be overridden by the contents of this README. In case of doubt, please check with C4 staff.



