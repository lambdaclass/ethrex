"""Just enough EVM to run RECENT_ROOT_CODE's two operations against the EIP-8272 reference
vector and a few negative cases. Not a general interpreter: exactly the opcodes the runtime uses."""
import sys
from eth_hash.auto import keccak
MASK = (1 << 256) - 1
def run(code, calldata, storage, slotnum, caller=1, value=0, static=False):
    st, mem, pc = [], bytearray(0x200), 0
    while True:
        op = code[pc]
        if 0x60 <= op <= 0x7f:
            n = op - 0x5f; st.append(int.from_bytes(code[pc+1:pc+1+n], "big")); pc += 1 + n; continue
        if 0x80 <= op <= 0x8f: st.append(st[-(op - 0x7f)]); pc += 1; continue
        if 0x90 <= op <= 0x9f:
            i = op - 0x8f; st[-1], st[-1-i] = st[-1-i], st[-1]; pc += 1; continue
        if op == 0x00: return ("stop", storage)
        if op == 0xfd: st.pop(); st.pop(); return ("revert", storage)
        if op == 0x5b: pc += 1; continue
        if op == 0x56: pc = st.pop(); continue
        if op == 0x57:
            dest, cond = st.pop(), st.pop(); pc = dest if cond else pc + 1; continue
        if op == 0x01: a, b = st.pop(), st.pop(); st.append((a + b) & MASK)
        elif op == 0x03: a, b = st.pop(), st.pop(); st.append((a - b) & MASK)
        elif op == 0x06: a, b = st.pop(), st.pop(); st.append(0 if b == 0 else a % b)
        elif op == 0x10: a, b = st.pop(), st.pop(); st.append(1 if a < b else 0)
        elif op == 0x11: a, b = st.pop(), st.pop(); st.append(1 if a > b else 0)
        elif op == 0x14: a, b = st.pop(), st.pop(); st.append(1 if a == b else 0)
        elif op == 0x15: st.append(1 if st.pop() == 0 else 0)
        elif op == 0x16: a, b = st.pop(), st.pop(); st.append(a & b)
        elif op == 0x1c: sh, v = st.pop(), st.pop(); st.append(v >> sh)
        elif op == 0x20:
            off, ln = st.pop(), st.pop(); st.append(int.from_bytes(keccak(bytes(mem[off:off+ln])), "big"))
        elif op == 0x33: st.append(caller)
        elif op == 0x34: st.append(value)
        elif op == 0x35:
            off = st.pop(); st.append(int.from_bytes((calldata[off:off+32] + b"\0"*32)[:32], "big"))
        elif op == 0x36: st.append(len(calldata))
        elif op == 0x37:
            m, d, ln = st.pop(), st.pop(), st.pop(); chunk = (calldata[d:d+ln] + b"\0"*ln)[:ln]; mem[m:m+ln] = chunk
        elif op == 0x4b: st.append(slotnum)
        elif op == 0x51: off = st.pop(); st.append(int.from_bytes(mem[off:off+32], "big"))
        elif op == 0x52: off, v = st.pop(), st.pop(); mem[off:off+32] = v.to_bytes(32, "big")
        elif op == 0x54: k = st.pop(); st.append(storage.get(k, 0))
        elif op == 0x55:
            k, v = st.pop(), st.pop()
            if static: return ("revert", storage)
            storage[k] = v
        else: raise SystemExit(f"unhandled opcode {op:#x} at {pc}")
        pc += 1
code = bytes.fromhex(open(sys.argv[1]).read().strip().removeprefix("0x"))
# reference vector: source 0x…01, salt 0, slot 1, root 2, current_slot 2
source = 1; salt = bytes(32); root = (2).to_bytes(32, "big")
res, storage = run(code, salt + root, {}, slotnum=1, caller=source)
assert res == "stop" and len(storage) == 1, res
key, val = next(iter(storage.items()))
print("write: storage_key", hex(key)[:20], "entry_hash", hex(val)[:20])
assert hex(key) == "0x5f027aa1cbe2df279bf6518edd4b44ea5409fd800189ec35224e10ab05e574c3", "storage_key differs from the EIP vector"
assert hex(val) == "0xa0d1254c851be5a133b4c9a9e300f5602fc0f43dbe65aa6a66930d4ca0a51b8" or hex(val) == "0x0a0d1254c851be5a133b4c9a9e300f5602fc0f43dbe65aa6a66930d4ca0a51b8".replace("0x0","0x"), "entry_hash differs"
source_id = bytes.fromhex("b9382d35273c75a50631a3e84d3c75ec9266e2b18c35a627e16cdbf26a18ca85")
tup = lambda slot, r: source_id + slot.to_bytes(8, "big") + r
cases = [
 ("valid tuple, current 2",            tup(1, root),            2, "stop"),
 ("two copies of the tuple",           tup(1, root)*2,          2, "stop"),
 ("wrong root",                        tup(1, (3).to_bytes(32,"big")), 2, "revert"),
 ("slot == current",                   tup(1, root),            1, "revert"),
 ("age 8191 (edge ok)",                tup(1, root),            8192, "stop"),
 ("age 8192 (expired)",                tup(1, root),            8193, "revert"),
 ("empty calldata",                    b"",                     2, "revert"),
 ("71 bytes",                          tup(1, root)[:71],       2, "revert"),
 ("73 bytes",                          tup(1, root)+b"\0",      2, "revert"),
 ("17 tuples",                         tup(1, root)*17,         2, "revert"),
 ("16 tuples",                         tup(1, root)*16,         2, "stop"),
 ("validate with value",               tup(1, root),            2, "revert"),
]
ok = True
for name, cd, slot, want in cases:
    got, _ = run(code, cd, dict(storage), slotnum=slot, value=(1 if name=="validate with value" else 0), static=True)
    flag = "PASS" if got == want else "FAIL"; ok &= got == want
    print(f"  {flag}  {name}: {got}")
print("all cases pass" if ok else "SOME CASES FAILED"); sys.exit(0 if ok else 1)
