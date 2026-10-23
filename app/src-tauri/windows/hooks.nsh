; PageWeave installer hooks (decision D-023).
;
; The full installer carries the translation languages under $INSTDIR\translate. They are moved to
; the user's data folder, where languages downloaded in the app also live, so updates (which use
; the small installer) and reinstalls keep them. In the small installer there is nothing to move.
!macro NSIS_HOOK_POSTINSTALL
  ${If} ${FileExists} "$INSTDIR\translate\*.*"
    CreateDirectory "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\translate"
    CopyFiles /SILENT "$INSTDIR\translate\*.*" "$LOCALAPPDATA\io.github.kuyamcliff.pageweave\translate"
    RMDir /r "$INSTDIR\translate"
  ${EndIf}
!macroend
