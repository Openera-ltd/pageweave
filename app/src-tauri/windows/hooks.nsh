; PageWeave installer hooks (decisions D-023, D-026).
;
; The full installer carries the translation models (translate\) and LibreOffice (office\) under
; $INSTDIR. They are moved to the user's data folder, where downloads from the app also go, so
; updates (which use the small installer) and reinstalls keep them. A rename is instant on the same
; drive; when the destination exists (a reinstall), the files are copied over it instead. In the
; small installer there is nothing to move.
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

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro PW_MOVE_TO_DATA "translate"
  !insertmacro PW_MOVE_TO_DATA "office"
!macroend
