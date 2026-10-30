@echo off
chcp 65001 >nul
title tor2vid
cd /d "%~dp0"
set TOR2VID_LOCAL=1
"%~dp0node\node.exe" "%~dp0server.js"
echo.
echo tor2vid остановлен. Нажмите любую клавишу, чтобы закрыть окно.
pause >nul
