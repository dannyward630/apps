// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"regexp"
	"strings"
	"sync"
)

const maxSDKStdoutLineBytes = 1024

var (
	sdkStartupLine = regexp.MustCompile(`^API server started successfully on: [A-Za-z0-9.:-]+:-?[0-9]+$`)
	sdkClientsLine = regexp.MustCompile(`^server clients and configuration updated: [0-9]+ clients \([0-9]+ auth entries \+ [0-9]+ Gemini API keys \+ [0-9]+ Claude API keys \+ [0-9]+ Codex keys \+ [0-9]+ Vertex-compat \+ [0-9]+ OpenAI-compat\)$`)
	sdkStdout      *sdkStdoutPipe
)

type sdkStdoutPipe struct {
	writer    *os.File
	done      chan struct{}
	closeOnce sync.Once
}

// captureSDKStdout works around direct fmt.Printf calls in CLIProxyAPI v6.10.9.
// The bridge captures stdout only. Remove it when the pinned SDK stops writing
// startup and client-update lines to standard output.
func captureSDKStdout(logger *slog.Logger) error {
	reader, writer, err := os.Pipe()
	if err != nil {
		return fmt.Errorf("capture SDK stdout: %w", err)
	}
	pipe := &sdkStdoutPipe{writer: writer, done: make(chan struct{})}
	sdkStdout = pipe
	os.Stdout = writer
	go func() {
		defer close(pipe.done)
		defer reader.Close()
		relaySDKStdout(reader, logger)
	}()
	return nil
}

// CloseSDKStdout drains the captured SDK output before the process exits.
func CloseSDKStdout() {
	if sdkStdout == nil {
		return
	}
	sdkStdout.closeOnce.Do(func() {
		_ = sdkStdout.writer.Close()
		<-sdkStdout.done
	})
}

func relaySDKStdout(source io.Reader, logger *slog.Logger) {
	reader := bufio.NewReader(source)
	line := make([]byte, 0, maxSDKStdoutLineBytes)
	tooLong := false
	for {
		fragment, err := reader.ReadSlice('\n')
		if len(line)+len(fragment) > maxSDKStdoutLineBytes {
			tooLong = true
		}
		remaining := maxSDKStdoutLineBytes - len(line)
		if remaining > len(fragment) {
			remaining = len(fragment)
		}
		line = append(line, fragment[:remaining]...)
		if err == nil || errors.Is(err, io.EOF) {
			if len(line) > 0 || tooLong {
				logSDKStdoutLine(logger, strings.TrimSpace(string(line)), tooLong)
			}
			line = line[:0]
			tooLong = false
		}
		if errors.Is(err, io.EOF) {
			return
		}
		if err != nil && !errors.Is(err, bufio.ErrBufferFull) {
			logger.Warn("SDK stdout capture stopped", "error", err)
			return
		}
	}
}

func logSDKStdoutLine(logger *slog.Logger, line string, tooLong bool) {
	if !tooLong && (sdkStartupLine.MatchString(line) || sdkClientsLine.MatchString(line)) {
		logger.Debug(line)
		return
	}
	// Unknown SDK output may contain a model body or credential. Keep only a
	// bounded event, without the line content, until the pinned SDK is reviewed.
	logger.Debug("SDK stdout line omitted", "reason", "unrecognized_or_long")
}
