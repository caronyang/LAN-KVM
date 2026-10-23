@echo off
rem Run net-check.ps1 without changing the machine execution policy.
rem Usage: run-net-check.cmd -Mode listen -Port 45871 -AddFirewallRule
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0net-check.ps1" %*
