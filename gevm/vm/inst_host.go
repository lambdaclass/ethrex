package vm

import (
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// logNImpl implements LOG0-LOG4: creates a log entry with N topics.
func logNImpl(interp *Interpreter, host Host, n int) {
	// Static gas: GasLog = 375 (not charged in switch dispatch).
	if !interp.Gas.RecordCost(spec.GasLog) {
		interp.HaltOOG()
		return
	}
	if interp.RuntimeFlag.IsStatic {
		interp.Halt(InstructionResultStateChangeDuringStaticCall)
		return
	}
	offsetVal, lenVal, ok := interp.Stack.Pop2()
	if !ok {
		interp.HaltUnderflow()
		return
	}
	// Pop topics into pre-allocated scratch on interpreter (avoids 128-byte stack copy).
	topics := &interp.TopicsScratch
	for i := 0; i < n; i++ {
		val, ok := interp.Stack.Pop()
		if !ok {
			interp.HaltUnderflow()
			return
		}
		topics[i] = val.ToBytes32()
	}
	length, ok := interp.asUsizeOrFail(lenVal)
	if !ok {
		return
	}
	// Dynamic gas: log cost
	cost := interp.GasParams.LogCost(uint8(n), uint64(length))
	if !interp.Gas.RecordCost(cost) {
		interp.HaltOOG()
		return
	}
	var data types.Bytes
	if length != 0 {
		offset, ok := interp.asUsizeOrFail(offsetVal)
		if !ok {
			return
		}
		if !interp.ResizeMemory(offset, length) {
			return
		}
		// Allocate from arena (released at transaction end, same lifetime as journal logs).
		data = interp.ReturnAlloc.Alloc(length)
		copy(data, interp.Memory.Slice(offset, length))
	}
	addr := interp.Input.TargetAddress
	if interp.Journal != nil {
		log := interp.Journal.AllocLog()
		log.Address = addr
		log.Topics = *topics
		log.NumTopics = uint8(n)
		log.Data = data
	} else {
		host.Log(addr, topics, n, data)
	}
}
