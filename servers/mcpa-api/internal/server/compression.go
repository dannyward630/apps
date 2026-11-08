// SPDX-License-Identifier: FSL-1.1-ALv2

package server

import (
	"compress/gzip"
	"errors"
	"io"

	"connectrpc.com/connect"
)

var errDecodedBodyLimit = errors.New("decoded management message exceeds 262144 bytes")

// boundedGzipReader stops after the first byte over the decoded limit.
// Connect-Go otherwise drains the rest of an oversized gzip message to reuse
// the connection. This wrapper avoids decompressing an attacker-sized suffix.
// Remove it when Connect-Go can reject oversized messages without draining.
type boundedGzipReader struct {
	reader *gzip.Reader
	read   int
}

func (r *boundedGzipReader) Reset(src io.Reader) error {
	r.read = 0
	return r.reader.Reset(src)
}

func (r *boundedGzipReader) Read(p []byte) (int, error) {
	if r.read > managementMessageMaxBytes {
		return 0, errDecodedBodyLimit
	}
	remaining := managementMessageMaxBytes + 1 - r.read
	if len(p) > remaining {
		p = p[:remaining]
	}
	n, err := r.reader.Read(p)
	r.read += n
	return n, err
}

func (r *boundedGzipReader) Close() error { return r.reader.Close() }

func managementCompression() connect.HandlerOption {
	return connect.WithCompression("gzip",
		func() connect.Decompressor { return &boundedGzipReader{reader: new(gzip.Reader)} },
		func() connect.Compressor { return gzip.NewWriter(io.Discard) })
}
