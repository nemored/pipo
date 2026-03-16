package store

import "sync/atomic"

type ErrorObserver interface {
	ObserveDBError(operation string, err error)
}

var dbErrorObserver atomic.Value

func SetErrorObserver(o ErrorObserver) {
	dbErrorObserver.Store(o)
}

func observeDBError(operation string, err error) {
	if err == nil {
		return
	}
	v := dbErrorObserver.Load()
	if v == nil {
		return
	}
	v.(ErrorObserver).ObserveDBError(operation, err)
}
