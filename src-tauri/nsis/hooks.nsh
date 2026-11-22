!macro NSIS_HOOK_POSTINSTALL
  ${IfNot} ${FileExists} "$APPDATA\PhaseMiX\.no_autostart"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe" --tray'
  ${EndIf}
!macroend
