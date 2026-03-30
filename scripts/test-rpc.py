#!/usr/bin/env python3
"""
Telos reth v1.11.3 RPC test suite.

Tests RPC correctness by:
1. Basic RPC methods (block, tx, receipt, balance, code, logs)
2. Comparison against a reference RPC endpoint
3. Edge cases (empty blocks, contract calls, traces)

Usage:
    python3 test-rpc.py --rpc http://localhost:8557
    python3 test-rpc.py --rpc http://localhost:8557 --ref https://rpc.testnet.telos.net/evm
"""

import argparse
import json
import sys
import time
import requests
from collections import defaultdict

# ─── Helpers ───────────────────────────────────────────────────

def rpc(url, method, params=None):
    """Send a JSON-RPC request."""
    payload = {
        "jsonrpc": "2.0",
        "method": method,
        "params": params or [],
        "id": 1,
    }
    try:
        r = requests.post(url, json=payload, timeout=30)
        data = r.json()
        if "error" in data:
            return None, data["error"]
        return data.get("result"), None
    except Exception as e:
        return None, str(e)

def hex_to_int(h):
    if h is None:
        return None
    if isinstance(h, int):
        return h
    return int(h, 16)

# ─── Test Functions ────────────────────────────────────────────

results = defaultdict(list)

def test(name, passed, detail=""):
    status = "✅ PASS" if passed else "❌ FAIL"
    results["pass" if passed else "fail"].append(name)
    msg = f"  {status}: {name}"
    if detail:
        msg += f" — {detail}"
    print(msg)
    return passed

def test_block_number(url):
    """Test eth_blockNumber returns a valid number."""
    print("\n📦 eth_blockNumber")
    result, err = rpc(url, "eth_blockNumber")
    if err:
        return test("eth_blockNumber", False, f"error: {err}")
    block = hex_to_int(result)
    return test("eth_blockNumber", block is not None and block > 0, f"block={block:,}")

def test_get_block(url):
    """Test eth_getBlockByNumber for latest and a specific block."""
    print("\n📦 eth_getBlockByNumber")
    
    # Latest block
    result, err = rpc(url, "eth_getBlockByNumber", ["latest", True])
    if err or result is None:
        test("getBlock(latest)", False, f"error: {err}")
        return False
    
    block_num = hex_to_int(result.get("number"))
    has_hash = result.get("hash") is not None and len(result["hash"]) == 66
    has_parent = result.get("parentHash") is not None
    has_timestamp = result.get("timestamp") is not None
    has_gas_limit = result.get("gasLimit") is not None
    
    test("getBlock(latest) returns block", block_num is not None and block_num > 0, f"number={block_num:,}")
    test("getBlock has hash", has_hash, result.get("hash", "")[:18] + "...")
    test("getBlock has parentHash", has_parent)
    test("getBlock has timestamp", has_timestamp)
    test("getBlock has gasLimit", has_gas_limit)
    
    tx_count = len(result.get("transactions", []))
    test("getBlock has transactions array", isinstance(result.get("transactions"), list), f"txs={tx_count}")
    
    return True

def test_chain_id(url):
    """Test eth_chainId returns correct value."""
    print("\n🔗 eth_chainId")
    result, err = rpc(url, "eth_chainId")
    if err:
        return test("eth_chainId", False, f"error: {err}")
    chain_id = hex_to_int(result)
    # 41 = testnet, 40 = mainnet
    valid = chain_id in (40, 41)
    return test("eth_chainId", valid, f"chainId={chain_id}")

def test_gas_price(url):
    """Test eth_gasPrice returns a value."""
    print("\n⛽ eth_gasPrice")
    result, err = rpc(url, "eth_gasPrice")
    if err:
        return test("eth_gasPrice", False, f"error: {err}")
    gas = hex_to_int(result)
    return test("eth_gasPrice", gas is not None and gas >= 0, f"gasPrice={gas}")

def test_get_balance(url):
    """Test eth_getBalance for known addresses."""
    print("\n💰 eth_getBalance")
    
    # Zero address should have some balance or at least not error
    result, err = rpc(url, "eth_getBalance", ["0x0000000000000000000000000000000000000000", "latest"])
    if err:
        test("getBalance(zero_addr)", False, f"error: {err}")
    else:
        bal = hex_to_int(result)
        test("getBalance(zero_addr)", bal is not None, f"balance={bal}")
    
    # WTLOS contract address — should exist
    result, err = rpc(url, "eth_getBalance", ["0xaE85Bf723A9e74d6c663dd226996AC1b8d075AA9", "latest"])
    if err:
        test("getBalance(WTLOS)", False, f"error: {err}")
    else:
        bal = hex_to_int(result)
        test("getBalance(WTLOS)", bal is not None, f"balance={bal}")

def test_get_code(url):
    """Test eth_getCode for known contracts."""
    print("\n📝 eth_getCode")
    
    # WTLOS contract on testnet
    result, err = rpc(url, "eth_getCode", ["0xaE85Bf723A9e74d6c663dd226996AC1b8d075AA9", "latest"])
    if err:
        return test("getCode(WTLOS)", False, f"error: {err}")
    has_code = result is not None and len(result) > 4
    return test("getCode(WTLOS)", has_code, f"code_length={len(result) if result else 0}")

def test_eth_call(url):
    """Test eth_call — call WTLOS name() function."""
    print("\n📞 eth_call")
    
    # name() = 0x06fdde03
    call_data = {
        "to": "0xaE85Bf723A9e74d6c663dd226996AC1b8d075AA9",
        "data": "0x06fdde03"
    }
    result, err = rpc(url, "eth_call", [call_data, "latest"])
    if err:
        return test("eth_call(WTLOS.name())", False, f"error: {err}")
    has_result = result is not None and len(result) > 4
    return test("eth_call(WTLOS.name())", has_result, f"result_length={len(result) if result else 0}")

def test_find_tx_block(url):
    """Find a recent block with transactions for further testing."""
    print("\n🔍 Finding block with transactions...")
    
    # Get latest block number
    result, err = rpc(url, "eth_blockNumber")
    if err:
        print(f"  ⚠️  Can't get block number: {err}")
        return None, None
    
    latest = hex_to_int(result)
    
    # Search backwards for a block with transactions
    for offset in range(0, 500):
        block_num = hex(latest - offset)
        result, err = rpc(url, "eth_getBlockByNumber", [block_num, True])
        if err or result is None:
            continue
        txs = result.get("transactions", [])
        if len(txs) > 0:
            tx_hash = txs[0]["hash"] if isinstance(txs[0], dict) else txs[0]
            print(f"  Found block {latest - offset} with {len(txs)} txs")
            return latest - offset, tx_hash
    
    print("  ⚠️  No blocks with transactions found in last 500 blocks")
    return None, None

def test_get_transaction(url, tx_hash):
    """Test eth_getTransactionByHash."""
    print(f"\n📤 eth_getTransactionByHash ({tx_hash[:18]}...)")
    
    result, err = rpc(url, "eth_getTransactionByHash", [tx_hash])
    if err:
        return test("getTransaction", False, f"error: {err}")
    if result is None:
        return test("getTransaction", False, "returned null")
    
    test("tx has hash", result.get("hash") == tx_hash)
    test("tx has from", result.get("from") is not None)
    test("tx has to", "to" in result)  # can be null for contract creation
    test("tx has value", result.get("value") is not None)
    test("tx has blockNumber", result.get("blockNumber") is not None)
    test("tx has gas", result.get("gas") is not None)
    return True

def test_get_receipt(url, tx_hash):
    """Test eth_getTransactionReceipt."""
    print(f"\n🧾 eth_getTransactionReceipt ({tx_hash[:18]}...)")
    
    result, err = rpc(url, "eth_getTransactionReceipt", [tx_hash])
    if err:
        return test("getReceipt", False, f"error: {err}")
    if result is None:
        return test("getReceipt", False, "returned null")
    
    test("receipt has transactionHash", result.get("transactionHash") == tx_hash)
    test("receipt has status", result.get("status") is not None)
    test("receipt has blockNumber", result.get("blockNumber") is not None)
    test("receipt has gasUsed", result.get("gasUsed") is not None)
    test("receipt has cumulativeGasUsed", result.get("cumulativeGasUsed") is not None)
    test("receipt has logs array", isinstance(result.get("logs"), list))
    
    status = hex_to_int(result.get("status"))
    test("receipt status is 0 or 1", status in (0, 1), f"status={status}")
    return True

def test_get_logs(url, block_num):
    """Test eth_getLogs for a specific block."""
    print(f"\n📋 eth_getLogs (block {block_num})")
    
    block_hex = hex(block_num)
    result, err = rpc(url, "eth_getLogs", [{"fromBlock": block_hex, "toBlock": block_hex}])
    if err:
        return test("getLogs", False, f"error: {err}")
    
    test("getLogs returns array", isinstance(result, list), f"logs={len(result)}")
    if len(result) > 0:
        log = result[0]
        test("log has address", log.get("address") is not None)
        test("log has topics", isinstance(log.get("topics"), list))
        test("log has data", log.get("data") is not None)
        test("log has blockNumber", log.get("blockNumber") is not None)
    return True

def test_net_version(url):
    """Test net_version."""
    print("\n🌐 net_version")
    result, err = rpc(url, "net_version")
    if err:
        return test("net_version", False, f"error: {err}")
    return test("net_version", result is not None, f"version={result}")

def test_syncing(url):
    """Test eth_syncing."""
    print("\n🔄 eth_syncing")
    result, err = rpc(url, "eth_syncing")
    if err:
        return test("eth_syncing", False, f"error: {err}")
    # False means fully synced, object means still syncing
    test("eth_syncing", result is not None or result == False, f"syncing={result}")
    return True

# ─── Comparison Tests ──────────────────────────────────────────

def test_comparison(url, ref_url):
    """Compare responses between test and reference RPC."""
    print(f"\n🔀 Comparison Tests (ref: {ref_url})")
    
    # Get a block number that both should have
    result, err = rpc(url, "eth_blockNumber")
    if err:
        print(f"  ⚠️  Can't get test block number: {err}")
        return
    test_head = hex_to_int(result)
    
    result, err = rpc(ref_url, "eth_blockNumber")
    if err:
        print(f"  ⚠️  Can't get ref block number: {err}")
        return
    ref_head = hex_to_int(result)
    
    # Use the lower of the two
    compare_block = min(test_head, ref_head) - 10  # slight offset for safety
    print(f"  Comparing at block {compare_block:,} (test head: {test_head:,}, ref head: {ref_head:,})")
    
    # Compare block
    block_hex = hex(compare_block)
    test_block, _ = rpc(url, "eth_getBlockByNumber", [block_hex, False])
    ref_block, _ = rpc(ref_url, "eth_getBlockByNumber", [block_hex, False])
    
    if test_block and ref_block:
        test("block hash matches", test_block.get("hash") == ref_block.get("hash"),
             f"test={test_block.get('hash','')[:18]}... ref={ref_block.get('hash','')[:18]}...")
        test("block parentHash matches", test_block.get("parentHash") == ref_block.get("parentHash"))
        test("block gasUsed matches", test_block.get("gasUsed") == ref_block.get("gasUsed"))
        test("block timestamp matches", test_block.get("timestamp") == ref_block.get("timestamp"))
        test("block txs count matches",
             len(test_block.get("transactions", [])) == len(ref_block.get("transactions", [])))
    else:
        test("block comparison", False, "one or both blocks returned null")
    
    # Compare balance
    addr = "0x0000000000000000000000000000000000000000"
    test_bal, _ = rpc(url, "eth_getBalance", [addr, block_hex])
    ref_bal, _ = rpc(ref_url, "eth_getBalance", [addr, block_hex])
    test("balance matches", test_bal == ref_bal,
         f"test={test_bal} ref={ref_bal}")

# ─── Main ──────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Telos reth RPC test suite")
    parser.add_argument("--rpc", required=True, help="RPC URL to test")
    parser.add_argument("--ref", help="Reference RPC URL for comparison")
    args = parser.parse_args()
    
    print("=" * 60)
    print(f"🧪 Telos reth v1.11.3 RPC Test Suite")
    print(f"   Target: {args.rpc}")
    if args.ref:
        print(f"   Reference: {args.ref}")
    print(f"   Time: {time.strftime('%Y-%m-%d %H:%M:%S')}")
    print("=" * 60)
    
    # Basic tests
    test_block_number(args.rpc)
    test_chain_id(args.rpc)
    test_gas_price(args.rpc)
    test_net_version(args.rpc)
    test_syncing(args.rpc)
    test_get_block(args.rpc)
    test_get_balance(args.rpc)
    test_get_code(args.rpc)
    test_eth_call(args.rpc)
    
    # Find a block with transactions for tx-specific tests
    block_num, tx_hash = test_find_tx_block(args.rpc)
    if tx_hash:
        test_get_transaction(args.rpc, tx_hash)
        test_get_receipt(args.rpc, tx_hash)
    if block_num:
        test_get_logs(args.rpc, block_num)
    
    # Comparison tests
    if args.ref:
        test_comparison(args.rpc, args.ref)
    
    # Summary
    passed = len(results["pass"])
    failed = len(results["fail"])
    total = passed + failed
    
    print("\n" + "=" * 60)
    print(f"📊 Results: {passed}/{total} passed, {failed} failed")
    if failed > 0:
        print(f"\n❌ Failed tests:")
        for name in results["fail"]:
            print(f"   - {name}")
    print("=" * 60)
    
    return 0 if failed == 0 else 1

if __name__ == "__main__":
    sys.exit(main())
