@rem Starts winc from cmd or PowerShell. --launcher lets winc.exe start its window in the background,
@rem so the command returns instead of waiting for winc to exit.
@"%~dp0..\winc.exe" --launcher %*
