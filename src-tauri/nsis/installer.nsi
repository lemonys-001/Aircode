; AI Coding Agent — Native Windows x64 NSIS Installer Script
!include "MUI2.nsh"

Name "AI Coding Agent"
OutFile "..\..\dist\AI-Coding-Agent-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\AI-Coding-Agent"
InstallDirRegKey HKCU "Software\Aircode\AI-Coding-Agent" "InstallDir"
RequestExecutionLevel user
Unicode True

!define MUI_ICON "..\icons\icon.ico"
!define MUI_UNICON "..\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\AI-Coding-Agent.exe"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "AI Coding Agent Desktop" SecMain
  SetOutPath "$INSTDIR"
  File "..\..\dist\AI-Coding-Agent.exe"
  File "..\icons\icon.ico"

  WriteRegStr HKCU "Software\Aircode\AI-Coding-Agent" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\AI Coding Agent"
  CreateShortcut "$SMPROGRAMS\AI Coding Agent\AI Coding Agent.lnk" "$INSTDIR\AI-Coding-Agent.exe" "" "$INSTDIR\icon.ico"
  CreateShortcut "$DESKTOP\AI Coding Agent.lnk" "$INSTDIR\AI-Coding-Agent.exe" "" "$INSTDIR\icon.ico"

  ; Windows Add/Remove Programs Registration
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AICodingAgent" "DisplayName" "AI Coding Agent"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AICodingAgent" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AICodingAgent" "DisplayIcon" "$INSTDIR\AI-Coding-Agent.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AICodingAgent" "Publisher" "Aircode Desktop"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AICodingAgent" "DisplayVersion" "1.0.0"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\AI-Coding-Agent.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\AI Coding Agent\AI Coding Agent.lnk"
  RMDir "$SMPROGRAMS\AI Coding Agent"
  Delete "$DESKTOP\AI Coding Agent.lnk"

  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AICodingAgent"
  DeleteRegKey HKCU "Software\Aircode\AI-Coding-Agent"
SectionEnd
