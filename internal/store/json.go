package store

import "regexp"

// Matches Rust's config comment stripping regex: //[^\n\r]*
var lineComments = regexp.MustCompile(`//[^\n\r]*`)

// StripJSONComments removes single-line // comments for Rust-compat config parsing.
func StripJSONComments(in []byte) []byte {
	return lineComments.ReplaceAll(in, nil)
}
