; PageWeave installer hooks (decisions D-023, D-026).
;
; The full installer carries the translation models (translate\) and the pinned LibreOffice
; package (office-package\LibreOffice_<version>.msi). Both go to the user's data folder, where
; downloads from the app also go, so updates (which use the small installer) and reinstalls keep
; them. In the small installer there is nothing to move.
;
; translate\ is moved as is. A rename is instant on the same drive; when the destination exists (a
; reinstall), the files are copied over it instead.
!macro PW_MOVE_TO_DATA name
  ${If} ${FileExists} "$INSTDIR\${name}\*.*"
    CreateDirectory "$LOCALAPPDATA\io.github.kuyamcliff.pageweave"
    ClearErrors
    ${IfNot} ${FileExists} "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\${name}\*.*"
      Rename "$INSTDIR\${name}" "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\${name}"
    ${Else}
      SetErrors
    ${EndIf}
    ${If} ${Errors}
      CreateDirectory "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\${name}"
      CopyFiles /SILENT "$INSTDIR\${name}\*.*" "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\${name}"
      RMDir /r "$INSTDIR\${name}"
    ${EndIf}
  ${EndIf}
!macroend

; LibreOffice is carried as its own package (an unpacked copy would push the installer past the
; 2 GB that an NSIS installer can hold). The package is put where the app keeps a downloaded one,
; then the bundled command-line tool unpacks it exactly as Settings > Resources does: checksum
; checked against the pin, `msiexec /a` (files only, no administrator rights, nothing registered),
; the program started once before it is used. No network is needed because the package is already
; there. If unpacking fails, the package stays in the data folder and Settings > Resources finishes
; it later without downloading.
!macro PW_UNPACK_OFFICE
  ${If} ${FileExists} "$INSTDIR\office-package\*.msi"
    CreateDirectory "$LOCALAPPDATA\io.github.kuyamcliff.pageweave"
    FindFirst $0 $1 "$INSTDIR\office-package\*.msi"
    ${If} $1 != ""
      Delete "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\$1"
      ClearErrors
      Rename "$INSTDIR\office-package\$1" "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\$1"
      ${If} ${Errors}
        CopyFiles /SILENT "$INSTDIR\office-package\$1" "$LOCALAPPDATA\io.github.kuyamcliff.pageweave"
      ${EndIf}
    ${EndIf}
    FindClose $0
    RMDir /r "$INSTDIR\office-package"
    ${If} ${FileExists} "$INSTDIR\pageweave.exe"
      DetailPrint "Unpacking LibreOffice for Word, Excel and PowerPoint files..."
      ; nsExec runs the tool without a console window; it is a standard NSIS plugin, but the NSIS
      ; that the bundler downloads may leave it out, so fall back to ExecWait (same exit code).
      !if /FileExists "${NSISDIR}\Plugins\x86-unicode\nsExec.dll"
        nsExec::ExecToLog '"$INSTDIR\pageweave.exe" office --install'
        Pop $0
      !else
        ExecWait '"$INSTDIR\pageweave.exe" office --install' $0
      !endif
      DetailPrint "LibreOffice: $0"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro PW_MOVE_TO_DATA "translate"
  !insertmacro PW_UNPACK_OFFICE
!macroend
