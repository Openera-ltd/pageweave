// Office documents PageWeave opens by converting them to PDF with LibreOffice (the same list as
// OFFICE_EXTENSIONS in crates/pw-pdf/src/office.rs).
export const OFFICE_EXTENSIONS = [
  "docx", "docm", "doc", "dotx", "dotm", "dot", "rtf", "txt",
  "xlsx", "xlsm", "xlsb", "xls", "xltx", "xltm", "xlt", "csv",
  "pptx", "pptm", "ppt", "ppsx", "ppsm", "pps", "potx", "potm", "pot",
  "vsdx", "vsd", "pub",
  "odt", "ods", "odp", "odg", "html", "htm",
];

const OFFICE_EXT = new RegExp(`\\.(${OFFICE_EXTENSIONS.join("|")})$`, "i");

export function isOfficePath(p: string): boolean {
  return OFFICE_EXT.test(p);
}
