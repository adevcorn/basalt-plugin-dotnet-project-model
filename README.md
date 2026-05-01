# basalt-plugin-dotnet-project-model

Basalt tier-0 `project-model:dotnet` plugin.

It detects:

- `.sln`
- `.csproj`
- `.fsproj`
- `.vbproj`

and emits Basalt's generic project-model JSON schema for canvas metadata and
future review/routing features.

Current scope:

- SDK-style project metadata
- solution member discovery from `.sln`
- basic target-kind inference for app/library/test projects

Current limitations:

- no directory listing import, so discovery is driven by matched activation files
- ownership metadata is strongest when the workspace root is also the solution/project root
- no full MSBuild evaluation or solution-folder graph yet
