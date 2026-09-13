package cli

import (
	"fmt"
	"runtime"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/buildinfo"
	"github.com/starfield17/rhost/internal/output"
)

type versionInfo struct {
	Version       string `json:"version"`
	Commit        string `json:"commit"`
	BuildDate     string `json:"build_date"`
	GoVersion     string `json:"go_version"`
	SchemaVersion int    `json:"schema_version"`
}

func newVersionCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "version",
		Short: "Print version information",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			info := versionInfo{
				Version:       buildinfo.Version,
				Commit:        buildinfo.Commit,
				BuildDate:     buildinfo.BuildDate,
				GoVersion:     runtime.Version(),
				SchemaVersion: output.SchemaVersion,
			}
			if jsonFlag {
				writeEnvelope(output.Success("version", "", info))
				return nil
			}
			fmt.Printf("rhost %s\n", info.Version)
			fmt.Printf("commit:         %s\n", info.Commit)
			fmt.Printf("build date:     %s\n", info.BuildDate)
			fmt.Printf("go version:     %s\n", info.GoVersion)
			fmt.Printf("schema version: %d\n", info.SchemaVersion)
			return nil
		},
	}
}
