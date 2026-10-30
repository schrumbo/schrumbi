Unicode true
!include "MUI2.nsh"

Name "Schrumbi"
OutFile "Schrumbi-Setup-${VERSION}-windows-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\Schrumbi"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "..\icons\schrumbi.ico"
!define MUI_UNICON "..\icons\schrumbi.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\schrumbi.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\..\target\release\schrumbi.exe"
  File "..\icons\schrumbi.ico"
  CreateShortcut "$SMPROGRAMS\Schrumbi.lnk" "$INSTDIR\schrumbi.exe" "" "$INSTDIR\schrumbi.ico"
  CreateShortcut "$DESKTOP\Schrumbi.lnk" "$INSTDIR\schrumbi.exe" "" "$INSTDIR\schrumbi.ico"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Classes\Schrumbi.Markdown" "" "Markdown"
  WriteRegStr HKCU "Software\Classes\Schrumbi.Markdown\DefaultIcon" "" "$INSTDIR\schrumbi.ico"
  WriteRegStr HKCU "Software\Classes\Schrumbi.Markdown\shell\open\command" "" '"$INSTDIR\schrumbi.exe" "%1"'
  WriteRegStr HKCU "Software\Classes\.md\OpenWithProgids" "Schrumbi.Markdown" ""
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Schrumbi" "DisplayName" "Schrumbi"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Schrumbi" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Schrumbi" "DisplayIcon" "$INSTDIR\schrumbi.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Schrumbi" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Schrumbi" "Publisher" "Schrumbo"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\schrumbi.exe"
  Delete "$INSTDIR\schrumbi.ico"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\Schrumbi.lnk"
  Delete "$DESKTOP\Schrumbi.lnk"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Schrumbi"
  DeleteRegKey HKCU "Software\Classes\Schrumbi.Markdown"
  DeleteRegValue HKCU "Software\Classes\.md\OpenWithProgids" "Schrumbi.Markdown"
SectionEnd
