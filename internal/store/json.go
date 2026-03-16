package store

import "regexp"

var lineComments = regexp.MustCompile(`(?m)//.*$`)

// StripJSONComments removes single-line // comments for Rust-compat config parsing.
func StripJSONComments(in []byte) []byte {
	return lineComments.ReplaceAll(in, nil)
}
