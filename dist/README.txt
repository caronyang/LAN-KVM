LAN-KVM v0.1-local

Contents:
  lankvm-agent.exe     product skeleton: probe
  lankvm-devtools.exe  development tools (hooks/injection/PoC/clipboard/file tests)
  lankvm-service.exe   Windows service + SYSTEM session agent (debug build)
  config.txt           session agent config template
  install-service.ps1  install service (Administrator)
  uninstall-service.ps1
  run-local-tests.ps1  probe/abs/clipboard/file local tests

Quick start:
  1. Run .\run-local-tests.ps1 in a normal PowerShell.
  2. For Winlogon testing, run install-service.ps1 from an Administrator
     PowerShell with -ClientPort 45871, then run:
       .\lankvm-devtools.exe local-winlogon-test --delay 15 --motion 300
     and lock the screen within 15 seconds.

Security:
  This is an isolated-LAN test build. UDP is unencrypted and unauthenticated.
  The service runs as LocalSystem. Do not install on production machines.
  Apex One may quarantine the service/devtools binaries; use a test VM or
  an IT-approved hash allowlist.

See docs/07-local-verification.md for detailed tests and expected output.
