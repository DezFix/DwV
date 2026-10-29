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

  ; The magnet ProgID + capabilities make DwV appear in Windows' "Open with" list.
  WriteRegStr HKCU "Software\Classes\DwV.magnet" "" "URL:magnet Protocol"
  WriteRegStr HKCU "Software\Classes\DwV.magnet" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\DwV.magnet\DefaultIcon" "" "$INSTDIR\dwv.exe,0"
  WriteRegStr HKCU "Software\Classes\DwV.magnet\shell\open\command" "" '"$INSTDIR\dwv.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\magnet\OpenWithProgids" "DwV.magnet" ""

  WriteRegStr HKCU "Software\DwV\Capabilities" "ApplicationName" "DwV"
  WriteRegStr HKCU "Software\DwV\Capabilities" "ApplicationDescription" "DwV Download Manager"
  WriteRegStr HKCU "Software\DwV\Capabilities" "ApplicationIcon" "$INSTDIR\dwv.exe,0"
  WriteRegStr HKCU "Software\DwV\Capabilities\UrlAssociations" "magnet" "DwV.magnet"
  WriteRegStr HKCU "Software\DwV\Capabilities\UrlAssociations" "dwv" "dwv"
  WriteRegStr HKCU "Software\RegisteredApplications" "DwV" "Software\DwV\Capabilities"

  ; Start with Windows. The app can disable this later from its settings.
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "DwV" '"$INSTDIR\dwv.exe" --autostart'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey HKCU "Software\Classes\dwv"
  DeleteRegKey HKCU "Software\Classes\DwV.magnet"
  DeleteRegValue HKCU "Software\Classes\magnet\OpenWithProgids" "DwV.magnet"
  DeleteRegKey HKCU "Software\DwV\Capabilities"
  DeleteRegValue HKCU "Software\RegisteredApplications" "DwV"
  ReadRegStr $0 HKCU "Software\Classes\magnet\shell\open\command" ""
  StrCmp $0 '"$INSTDIR\dwv.exe" "%1"' 0 +2
    DeleteRegKey HKCU "Software\Classes\magnet"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "DwV"
!macroend
