package main

// These four closed enums are checked by TestStateExhaustiveness. Every switch
// on one must name every variant, even if it also has a defensive default.
type runOutcome uint8

const (
	runSucceeded runOutcome = iota
	runFailed
	runInterrupted
)

func outcome(ctxErr, err error) runOutcome {
	if ctxErr != nil {
		return runInterrupted
	}
	if err != nil {
		return runFailed
	}
	return runSucceeded
}

func (o runOutcome) failed() bool {
	switch o {
	case runSucceeded:
		return false
	case runFailed, runInterrupted:
		return true
	}
	panic("invalid Run outcome")
}

type claimState uint8

const (
	claimReleased claimState = iota
	claimHeld
	claimFailed
	claimClosed
)

type deliveryState uint8

const (
	deliveryReconcile deliveryState = iota
	deliveryPublish
	deliveryWatch
	deliveryRepair
	deliveryReady
)

type ciState uint8

const (
	ciAbsent ciState = iota
	ciPending
	ciPassed
	ciFailed
)
