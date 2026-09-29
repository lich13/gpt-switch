!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\gptSwitch.CCSwitch.0" "" "gpt-Switch CC Switch Link"
  WriteRegStr HKCU "Software\Classes\gptSwitch.CCSwitch.0" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\gptSwitch.CCSwitch.0\shell\open\command" "" '$\"$INSTDIR\gpt-switch.exe$\" $\"%1$\"'
  WriteRegStr HKCU "Software\gpt-Switch\LinkHandlers\0\Capabilities" "ApplicationName" "gpt-Switch"
  WriteRegStr HKCU "Software\gpt-Switch\LinkHandlers\0\Capabilities" "ApplicationDescription" "CC Switch links"
  WriteRegStr HKCU "Software\gpt-Switch\LinkHandlers\0\Capabilities\URLAssociations" "ccswitch" "gptSwitch.CCSwitch.0"
  WriteRegStr HKCU "Software\RegisteredApplications" "gpt-Switch Link gpt-Switch" "Software\gpt-Switch\LinkHandlers\0\Capabilities"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\gptSwitch.CCSwitch.0"
  DeleteRegKey HKCU "Software\Classes\gptSwitch.CCSwitch.1"
  DeleteRegKey HKCU "Software\Classes\gptSwitch.CCSwitch.2"
  DeleteRegKey HKCU "Software\gpt-Switch\LinkHandlers"
  DeleteRegValue HKCU "Software\RegisteredApplications" "gpt-Switch Link gpt-Switch"
  DeleteRegValue HKCU "Software\RegisteredApplications" "gpt-Switch Link CC Switch"
  DeleteRegValue HKCU "Software\RegisteredApplications" "gpt-Switch Link lich13studio"
!macroend
