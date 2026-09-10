// Package buildinfo carries version metadata that release builds inject via
// -ldflags. Development builds keep the placeholder values.
package buildinfo

// These variables are overridden at release time, e.g.:
//
//	go build -ldflags "\
//	  -X github.com/starfield17/rhost/internal/buildinfo.Version=0.1.0 \
//	  -X github.com/starfield17/rhost/internal/buildinfo.Commit=$(git rev-parse --short HEAD) \
//	  -X github.com/starfield17/rhost/internal/buildinfo.BuildDate=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
var (
	Version   = "dev"
	Commit    = "none"
	BuildDate = "unknown"
)
