package protocol

import (
	"context"
	"fmt"
	"sync"
)

// ProtocolError surfaces a server `error` event as a Go error.
type ProtocolError struct {
	Code      string
	Message   string
	RequestID string
}

func (e *ProtocolError) Error() string {
	if e.RequestID != "" {
		return fmt.Sprintf("smooth-agent: protocol error %s: %s (requestId=%s)", e.Code, e.Message, e.RequestID)
	}
	return fmt.Sprintf("smooth-agent: protocol error %s: %s", e.Code, e.Message)
}

// protocolErrorFromEvent builds a ProtocolError from an error event, tolerating
// either the nested data.error or the envelope-level error shape.
func protocolErrorFromEvent(ev ServerEvent) *ProtocolError {
	pe := &ProtocolError{Code: "INTERNAL_ERROR", Message: "Unknown protocol error", RequestID: ev.RequestID}
	if errEv, err := ev.AsError(); err == nil {
		if errEv.Data.Error.Code != "" {
			pe.Code = errEv.Data.Error.Code
			pe.Message = errEv.Data.Error.Message
		} else if errEv.Error != nil && errEv.Error.Code != "" {
			pe.Code = errEv.Error.Code
			pe.Message = errEv.Error.Message
		}
	}
	return pe
}

// MessageTurn is a single streaming send_message turn. Receive each intermediate
// event in arrival order from Events(), or block for the terminal eventual_response
// with Wait(ctx). HITL resumes (confirm_tool_action / verify_otp) for the same
// requestId flow back into the same turn.
//
//	turn := client.SendMessage(protocol.SendMessageParams{SessionID: id, Message: "hi"})
//	for ev := range turn.Events() {
//	    if ev.Type == protocol.EventStreamToken {
//	        tok, _ := ev.AsStreamToken()
//	        fmt.Print(tok.Token)
//	    }
//	}
//	final, err := turn.Wait(context.Background())
type MessageTurn struct {
	requestID string
	onClose   func()

	events chan ServerEvent

	mu        sync.Mutex
	done      bool
	final     *EventualResponse
	failErr   error
	settled   chan struct{} // closed once the turn finishes
	closeOnce sync.Once
}

func newMessageTurn(requestID string, onClose func()) *MessageTurn {
	return &MessageTurn{
		requestID: requestID,
		onClose:   onClose,
		events:    make(chan ServerEvent, 64),
		settled:   make(chan struct{}),
	}
}

// RequestID is the correlation ID this turn is keyed on.
func (t *MessageTurn) RequestID() string { return t.requestID }

// Events returns the channel of streamed events. It is closed when the turn ends
// (after the terminal event has been delivered, or on abort).
func (t *MessageTurn) Events() <-chan ServerEvent { return t.events }

// Wait blocks until the turn produces its terminal eventual_response, the turn
// fails (error event / transport close), or ctx is cancelled.
func (t *MessageTurn) Wait(ctx context.Context) (EventualResponse, error) {
	select {
	case <-t.settled:
		t.mu.Lock()
		defer t.mu.Unlock()
		if t.failErr != nil {
			return EventualResponse{}, t.failErr
		}
		if t.final != nil {
			return *t.final, nil
		}
		return EventualResponse{}, fmt.Errorf("smooth-agent: turn ended without a terminal response")
	case <-ctx.Done():
		return EventualResponse{}, ctx.Err()
	}
}

// push feeds an event into the turn. Called by the client dispatcher.
func (t *MessageTurn) push(ev ServerEvent) {
	t.mu.Lock()
	if t.done {
		t.mu.Unlock()
		return
	}
	t.mu.Unlock()

	// Deliver the event to consumers first so ordering is preserved.
	t.events <- ev

	switch ev.Type {
	case EventError:
		t.finish(nil, protocolErrorFromEvent(ev))
	case EventEventualResponse:
		final, err := ev.AsEventualResponse()
		if err != nil {
			t.finish(nil, err)
			return
		}
		t.finish(&final, nil)
	}
}

// abort force-closes the turn with an error (e.g. on disconnect).
func (t *MessageTurn) abort(err error) {
	t.finish(nil, err)
}

// finish settles the turn exactly once, recording the outcome and closing channels.
func (t *MessageTurn) finish(final *EventualResponse, err error) {
	t.mu.Lock()
	if t.done {
		t.mu.Unlock()
		return
	}
	t.done = true
	t.final = final
	t.failErr = err
	t.mu.Unlock()

	if t.onClose != nil {
		t.onClose()
	}
	t.closeOnce.Do(func() {
		close(t.events)
		close(t.settled)
	})
}
