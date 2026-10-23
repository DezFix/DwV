!macro NSIS_HOOK_POSTINSTALL
  ; URL protocols: make the browser able to launch DwV even before its first run.
  WriteRegStr HKCU "Software\Classes\dwv" "" "URL:dwv Protocol"
  WriteRegStr HKCU "Software\Classes\dwv" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\dwv\DefaultIcon" "" "$INSTDIR\dwv.exe,0"
  WriteRegStr HKCU "Software\Classes\dwv\shell\open\command" "" '"$INSTDIR\dwv.exe" "%1"'

  WriteRegStr HKCU "Software\Classes\magnet" "" "URL:magnet Protocol"
  WriteRegStr HKCU "Software\Classes\magnet" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\magnet\DefaultIcon" "" "$INSTDIR\dwv.exe,0"
  WriteRegStr HKCU "Software\Classes\magnet\shell\open\command" "" '"$INSTDIR\dwv.exe" "%1"'

  ; Start with Windows. The app can disable this later from its settings.
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "DwV" '"$INSTDIR\dwv.exe" --autostart'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey HKCU "Software\Classes\dwv"
  ReadRegStr $0 HKCU "Software\Classes\magnet\shell\open\command" ""
  StrCmp $0 '"$INSTDIR\dwv.exe" "%1"' 0 +2
    DeleteRegKey HKCU "Software\Classes\magnet"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "DwV"
!macroend
