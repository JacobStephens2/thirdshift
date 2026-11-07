package main_test

import (
	"fmt"
	"os"
	"slices"
	"strings"
	"testing"
)

// Independent bounded model; request mapping and limits: PICKUP-MODEL.md.
// Every edge is one remote request or an explicit local control/death marker.
type pickupPhase uint8

const (
	modelList pickupPhase = iota
	modelSelectHistory
	modelReserve
	modelRefreshIssue
	modelRefreshHistory
	modelClaimWrite
	modelSetup
	modelDispatch
	modelRunning
	modelEndRead
	modelEndWrite
	modelRelease
	modelDone
	modelCrashed
)

type pickupProtocol uint8

const (
	guarded pickupProtocol = iota
	WithoutClaimCount
	UnconditionalCreate
)

func (p pickupProtocol) String() string {
	return [...]string{"ADR-0017", "WithoutClaimCount", "UnconditionalCreate"}[p]
}

type modelLabels struct{ ready, claimed bool }
type pickupHost struct {
	phase                      pickupPhase
	generation                 uint8
	entry, ending              modelLabels
	dispatched, claimCompleted bool
	endingClosed               bool
}
type pickupState struct {
	hosts                    [3]pickupHost
	labels                   modelLabels
	reservation, dayObserved int8 // -1 means absent; otherwise the unique attempt/host.
	claimCount               uint8
	closed                   bool
}
type pickupEdge struct {
	state  pickupState
	action string
}

func pickupSuccessors(s pickupState, protocol pickupProtocol) []pickupEdge {
	var edges []pickupEdge
	add := func(next pickupState, action string, h int) {
		edges = append(edges, pickupEdge{next, fmt.Sprintf("%s(%d)", action, h)})
	}
	for h, host := range s.hosts {
		n := s
		next := &n.hosts[h]
		action := ""
		switch host.phase {
		case modelList:
			action = "ListReady"
			next.phase = modelDone
			if !s.closed && s.labels.ready && !s.labels.claimed {
				next.phase = modelSelectHistory
			}
		case modelSelectHistory:
			action = "SelectHistory"
			next.generation, next.phase = s.claimCount, modelReserve
			if s.labels.claimed {
				next.phase = modelDone
			}
		case modelReserve:
			action, next.phase = "CreateReservation", modelDone
			if s.reservation == -1 || protocol == UnconditionalCreate {
				n.reservation, next.phase = int8(h), modelRefreshIssue
			}
		case modelRefreshIssue:
			action = "RefreshIssue"
			next.entry, next.phase = s.labels, modelRelease
			if !s.closed && s.labels.ready && !s.labels.claimed {
				next.phase = modelRefreshHistory
			}
		case modelRefreshHistory:
			action, next.phase = "RefreshHistory", modelRelease
			if !s.labels.claimed && (protocol == WithoutClaimCount || s.claimCount == host.generation) {
				next.phase = modelClaimWrite
			}
		case modelClaimWrite:
			action = "PutClaimLabels"
			if !s.labels.claimed {
				n.claimCount++
			}
			n.labels = modelLabels{claimed: true}
			next.claimCompleted, next.phase = true, modelSetup
		case modelSetup:
			action, next.phase = "PrepareWorktree", modelDispatch
		case modelDispatch:
			action = "Dispatch"
			next.dispatched, next.phase = true, modelRunning
		case modelRunning:
			action, next.phase = "RunEndsWithoutOriginWork", modelEndRead
		case modelEndRead:
			action = "ReadEndingLabels"
			next.ending, next.phase = s.labels, modelRelease
			next.endingClosed = s.closed
			if s.labels.claimed {
				next.phase = modelEndWrite
			}
		case modelEndWrite:
			action = "RestoreEntryLabels"
			n.labels = modelLabels{ready: host.ending.ready || host.entry.ready, claimed: host.entry.claimed}
			if host.endingClosed {
				n.labels = modelLabels{ready: host.ending.ready}
			}
			next.phase = modelRelease
		case modelRelease:
			action = "ReleaseReservation"
			if s.reservation == int8(h) {
				n.reservation = -1
			}
			next.phase = modelDone
		case modelDone, modelCrashed:
			continue
		default:
			panic("unhandled Pickup phase")
		}
		add(n, action, h)
		n = s
		n.hosts[h].phase = modelCrashed
		add(n, "Crash", h)
		if host.phase == modelClaimWrite || host.phase == modelSetup || host.phase == modelDispatch {
			n = s
			n.hosts[h].phase = modelEndRead
			add(n, "FailBeforeDispatch", h)
		}
		if host.phase == modelRunning {
			n = s
			n.hosts[h].phase = modelRelease
			add(n, "KeepClaimWithOriginWork", h)
			n = s
			n.closed, n.hosts[h].phase = true, modelEndRead
			add(n, "IssueClosed", h)
		}
	}
	if s.reservation != -1 && s.dayObserved != s.reservation {
		n := s
		n.dayObserved = s.reservation
		edges = append(edges, pickupEdge{n, "DayReadsReservation"})
	}
	if s.dayObserved != -1 && s.reservation == s.dayObserved && s.hosts[s.dayObserved].phase == modelCrashed {
		n := s
		n.reservation = -1
		edges = append(edges, pickupEdge{n, "DayDeletesCrashedReservation"})
	}
	if s.labels.ready {
		n := s
		n.labels.ready = false
		edges = append(edges, pickupEdge{n, "DayWithdrawsReady"})
	} else if allDispatchedWorkEnded(s) {
		n := s
		n.labels = modelLabels{ready: true}
		n.closed = false
		edges = append(edges, pickupEdge{n, "DayHandsBack"})
	}
	return edges
}

func allDispatchedWorkEnded(s pickupState) bool {
	for _, host := range s.hosts {
		if host.dispatched && host.phase != modelRelease && host.phase != modelDone && host.phase != modelCrashed {
			return false
		}
	}
	return true
}

func uniquePickupGeneration(s pickupState, claimedOnly bool) bool {
	for h, host := range s.hosts {
		if !host.dispatched || (claimedOnly && !host.claimCompleted) {
			continue
		}
		for _, other := range s.hosts[:h] {
			if other.dispatched && (!claimedOnly || other.claimCompleted) && host.generation == other.generation {
				return false
			}
		}
	}
	return true
}

type pickupNode struct {
	state  pickupState
	parent int
	action string
}
type pickupExploration struct {
	states                      int
	dispatchTrace, claimedTrace []string
	reached                     map[string]bool
}

// FIFO discovery + visited-state deduplication makes the first violating
// trace shortest by action count. Parent links retain a replayable witness.
func explorePickup(protocol pickupProtocol) pickupExploration {
	initial := pickupState{labels: modelLabels{ready: true}, reservation: -1, dayObserved: -1}
	queue := []pickupNode{{state: initial, parent: -1}}
	seen := map[pickupState]bool{initial: true}
	result := pickupExploration{reached: map[string]bool{}}
	trace := func(index int) []string {
		var actions []string
		for queue[index].parent != -1 {
			actions = append(actions, queue[index].action)
			index = queue[index].parent
		}
		slices.Reverse(actions)
		return actions
	}
	for head := 0; head < len(queue); head++ {
		s := queue[head].state
		if !uniquePickupGeneration(s, false) && result.dispatchTrace == nil {
			result.dispatchTrace = trace(head)
		}
		if !uniquePickupGeneration(s, true) && result.claimedTrace == nil {
			result.claimedTrace = trace(head)
		}
		if result.dispatchTrace != nil && result.claimedTrace != nil {
			break // Both minimal witnesses found for a broken variant.
		}
		if s.hosts[0].phase == modelReserve && s.hosts[1].phase == modelReserve && s.hosts[2].phase == modelReserve {
			result.reached["three hosts select"] = true
		}
		for _, edge := range pickupSuccessors(s, protocol) {
			for _, host := range edge.state.hosts {
				if host.dispatched && host.claimCompleted && host.phase == modelRelease && edge.state.labels.ready && host.ending.claimed {
					result.reached["failed winner restores Ready"] = true
				}
			}
			if edge.action == "DayDeletesCrashedReservation" || edge.action == "DayHandsBack" || strings.HasPrefix(edge.action, "RestoreEntryLabels") || strings.HasPrefix(edge.action, "FailBeforeDispatch") || strings.HasPrefix(edge.action, "IssueClosed") {
				result.reached[strings.SplitN(edge.action, "(", 2)[0]] = true
			}
			if !seen[edge.state] {
				seen[edge.state] = true
				queue = append(queue, pickupNode{edge.state, head, edge.action})
			}
		}
	}
	result.states = len(seen)
	return result
}

func TestPickupModelSafety(t *testing.T) {
	result := explorePickup(guarded)
	t.Logf("ADR-0017: %d unique states", result.states)
	if result.dispatchTrace != nil {
		t.Fatalf("one dispatch per Ready generation: %v", result.dispatchTrace)
	}
	if result.claimedTrace != nil {
		t.Fatalf("one claimed dispatch per Ready generation: %v", result.claimedTrace)
	}
	for _, name := range []string{"three hosts select", "DayDeletesCrashedReservation", "DayHandsBack", "failed winner restores Ready", "RestoreEntryLabels", "FailBeforeDispatch", "IssueClosed"} {
		if !result.reached[name] {
			t.Errorf("vacuous safety: did not reach %s", name)
		}
	}
}

func TestPickupModelMutations(t *testing.T) {
	for _, protocol := range []pickupProtocol{WithoutClaimCount, UnconditionalCreate} {
		t.Run(protocol.String(), func(t *testing.T) {
			result := explorePickup(protocol)
			t.Logf("%s: %d states; shortest dispatch trace (%d): %v; shortest claimed trace (%d): %v", protocol, result.states, len(result.dispatchTrace), result.dispatchTrace, len(result.claimedTrace), result.claimedTrace)
			if result.dispatchTrace == nil || result.claimedTrace == nil {
				t.Fatal("mutation did not violate both safety properties")
			}
			wantLength := 19
			if protocol == UnconditionalCreate {
				wantLength = 16
			}
			if len(result.dispatchTrace) != wantLength || len(result.claimedTrace) != wantLength {
				t.Fatalf("minimal trace length changed: want %d; replay and review the request mapping", wantLength)
			}
			s := pickupState{labels: modelLabels{ready: true}, reservation: -1, dayObserved: -1}
			for index, action := range result.claimedTrace {
				found := false
				for _, edge := range pickupSuccessors(s, protocol) {
					if edge.action == action {
						s, found = edge.state, true
						break
					}
				}
				if !found || (index < wantLength-1 && !uniquePickupGeneration(s, true)) {
					t.Fatalf("invalid minimal witness at %s", action)
				}
			}
			if uniquePickupGeneration(s, true) {
				t.Fatal("replayed witness did not violate claimed-dispatch safety")
			}
			// Optional executable negative assertions; ordinary CI requires the
			// counterexamples and remains green while guarding sensitivity.
			if os.Getenv("PICKUP_MODEL_VARIANT") == protocol.String() {
				t.Fatalf("one claimed dispatch per Ready generation: %v", result.claimedTrace)
			}
		})
	}
}
