@echo off
rem Start power-hub with verbose logging (Debug level, serial raw frames included).
rem Put this file in the same folder as power-hub.exe, then double-click it.
rem (Keep this file ASCII-only: cmd mis-parses UTF-8 Chinese under the GBK codepage.)
set POWER_HUB_LOG=debug
start "" "%~dp0power-hub.exe"
