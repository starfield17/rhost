package fileops

import _ "embed"

// RemoteProgram is a one-shot helper sent through the existing SSH transport.
//
//go:embed remote.py
var RemoteProgram string
