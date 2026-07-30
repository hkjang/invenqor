Invenqor Asset Inventory Agent - Windows
========================================

Install (elevated PowerShell, in this directory):

    .\scripts\install.ps1

Then set the Server URL and restart:

    notepad "$env:ProgramData\Invenqor\config.toml"
    Restart-Service invenqor-agent

Verify:

    & "$env:ProgramFiles\Invenqor\invenqor-agent.exe" `
        --config "$env:ProgramData\Invenqor\config.toml" --diagnose

What gets installed
-------------------

    %ProgramFiles%\Invenqor\invenqor-agent.exe   the service binary
    %ProgramData%\Invenqor\config.toml           configuration
    %ProgramData%\Invenqor\state\                identity, inventory hash, queue

Both directories are restricted to SYSTEM and Administrators. The state
directory holds the device credential and the configuration can hold an
enrollment token, so neither may be readable by ordinary users.

The service runs as LocalSystem, starts delayed-automatic, and connects outbound
only - it opens no listening port. It needs LocalSystem to read the Service
Control Manager, the installed software of every loaded user profile, and the
network adapter configuration.

Upgrading
---------

Run install.ps1 from the new package. It stops the service, replaces the binary,
keeps the configuration and the undelivered queue, repairs the permissions and
starts the service again.

Signed automatic updates, when enabled, need no helper: a running executable
cannot be overwritten on Windows but it can be renamed, so the agent moves the
old binary to invenqor-agent.exe.previous, writes the new one in its place, runs
it once to confirm it starts and reports the expected version, then stops so the
Service Control Manager's recovery action restarts it on the new file. If the new
binary fails that check it is discarded and the running one is untouched.

Uninstalling
------------

    .\scripts\uninstall.ps1              keeps the configuration and queue
    .\scripts\uninstall.ps1 -RemoveData  deletes them as well

Full documentation: docs\ADMIN_GUIDE.md and docs\SERVER_INSTALLATION.md
