package transports

import "errors"

type terminalError struct{ err error }

func (e *terminalError) Error() string { return e.err.Error() }
func (e *terminalError) Unwrap() error { return e.err }

func asTerminal(err error) error {
	if err == nil {
		return nil
	}
	return &terminalError{err: err}
}

func isTerminal(err error) bool {
	var te *terminalError
	return errors.As(err, &te)
}
