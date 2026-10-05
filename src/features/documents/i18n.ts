import type { AppError, UiLocale } from '../../lib/types';
import { localizedError } from '../../lib/i18n';
const vi = {
  projectFilter: 'Dự án QuickTranslator', documentFilter: 'QuickTranslator / văn bản / HTML', allFiles: 'Tất cả tệp',
  unsavedTitle: 'Lưu thay đổi?', unsaved: 'Tài liệu có thay đổi chưa lưu. Lưu trước khi tiếp tục?', save: 'Lưu', discard: 'Bỏ thay đổi', cancel: 'Hủy', closeDocument: 'Đóng cửa sổ',
  importTitle: 'Mở / nhập tài liệu', encoding: 'Bảng mã', detected: 'Đã nhận diện bảng mã; kiểm tra trước khi nhập.', preview: 'Xem trước', import: 'Mở tài liệu', auto: 'Tự động (BOM / UTF-8 / nhận diện)', warnings: 'Lưu ý nhập / xuất',
  fileError: 'Không thể hoàn tất thao tác tệp. Tài liệu hiện tại được giữ nguyên.', docUnsupported: 'Không hỗ trợ Word .doc nhị phân. Hãy chuyển sang TXT hoặc HTML rồi mở lại.',
  invalidProject: 'Tệp dự án không hợp lệ hoặc dùng phiên bản chưa hỗ trợ. Tài liệu hiện tại được giữ nguyên.', invalidEncoding: 'Không thể giải mã văn bản; chọn đúng bảng mã và xem trước lại.',
  exportTitle: 'Xuất tài liệu', format: 'Định dạng', columns: 'Các cột theo thứ tự', blankLines: 'Số dòng trống giữa đoạn', up: 'Lên', down: 'Xuống', export: 'Xuất…', originalRtf: 'Xuất RTF gốc…',
  recoveryTitle: 'Khôi phục công việc chưa lưu', recoveryNotice: 'Các bản khôi phục được lưu riêng theo tài liệu. Bỏ một bản không xóa các bản khác.', recover: 'Khôi phục', later: 'Để sau', recoveryFailed: 'Không thể đọc / lưu bản khôi phục. Công việc trong biên tập vẫn được giữ.',
  saved: 'Đã lưu tài liệu.', exported: 'Đã xuất tài liệu.', syncWarning: 'Đã thay tệp; hệ thống không xác nhận đồng bộ thư mục.', multipleDrops: 'Chỉ mở một tài liệu mỗi lần. Hãy thả một tệp.',
  busy: 'Đang xử lý tệp…', settingsInvalid: 'Tùy chọn hỏng được giữ lại để khôi phục; đang dùng mặc định an toàn. Chưa ghi đè tệp.', replaceSettings: 'Lưu tùy chọn mới và lưu bản sao tệp hỏng', settingsError: 'Không thể nạp / lưu tùy chọn ứng dụng.',
  shortcuts: 'Phím tắt và đoạn mẫu', wordNext: 'Từ tiếp', wordPrevious: 'Từ trước', lineNext: 'Dòng tiếp', linePrevious: 'Dòng trước', paragraphNext: 'Đoạn tiếp', paragraphPrevious: 'Đoạn trước',
  shortcutHelp: 'Ctrl + phím cấu hình để di chuyển trên văn bản đã đối chiếu. Ctrl+0 chèn cách đọc; Ctrl+1…6 chèn nghĩa. F1…F9 chèn đoạn mẫu vào Việt. Alt+←/→ mở tài liệu liền kề; Alt+Shift+←/→ đổi thứ tự từ nháp.',
  typedHelp: 'Shortcuts.txt chỉ mở rộng từ vừa gõ trong Việt khi gõ dấu phân cách; không mở rộng lúc nhập IME hoặc dán hàng loạt.', snippets: 'Đoạn mẫu F1…F9', shortcutConflict: 'Phím đã dùng hoặc dành riêng cho thao tác ứng dụng.',
  cursor: 'Dòng / cột',
} as const;
export type DocumentMessage = keyof typeof vi;
const en: Record<DocumentMessage, string> = {
  projectFilter: 'QuickTranslator project', documentFilter: 'QuickTranslator / text / HTML', allFiles: 'All files',
  unsavedTitle: 'Save changes?', unsaved: 'This document has unsaved changes. Save before continuing?', save: 'Save', discard: 'Discard', cancel: 'Cancel', closeDocument: 'Close window',
  importTitle: 'Open / import document', encoding: 'Encoding', detected: 'Encoding was detected; review it before importing.', preview: 'Preview', import: 'Open document', auto: 'Automatic (BOM / UTF-8 / detection)', warnings: 'Import / export notes',
  fileError: 'The file operation failed. Your current document is preserved.', docUnsupported: 'Binary Word .doc input is not supported. Convert it to TXT or HTML and open that file.',
  invalidProject: 'The project is invalid or uses an unsupported version. Your current document is preserved.', invalidEncoding: 'Text could not be decoded. Choose the correct encoding and preview again.',
  exportTitle: 'Export document', format: 'Format', columns: 'Columns in output order', blankLines: 'Blank lines between paragraphs', up: 'Up', down: 'Down', export: 'Export…', originalRtf: 'Export original RTF…',
  recoveryTitle: 'Recover unsaved work', recoveryNotice: 'Recovery snapshots belong to individual documents. Discarding one leaves all other records intact.', recover: 'Recover', later: 'Later', recoveryFailed: 'Recovery could not be read / saved. Current editor work is preserved.',
  saved: 'Document saved.', exported: 'Document exported.', syncWarning: 'The file was replaced; the system could not confirm directory synchronization.', multipleDrops: 'Open one document at a time. Drop a single file.',
  busy: 'Working with document…', settingsInvalid: 'Invalid settings were preserved for recovery; safe defaults are in use. The file has not been overwritten.', replaceSettings: 'Save new settings and retain an invalid-file backup', settingsError: 'Application settings could not be loaded / saved.',
  shortcuts: 'Keyboard shortcuts and snippets', wordNext: 'Next word', wordPrevious: 'Previous word', lineNext: 'Next line', linePrevious: 'Previous line', paragraphNext: 'Next paragraph', paragraphPrevious: 'Previous paragraph',
  shortcutHelp: 'Ctrl + configured key navigates aligned text. Ctrl+0 inserts a reading; Ctrl+1…6 inserts a meaning. F1…F9 inserts a snippet into Vietnamese. Alt+←/→ opens sibling documents; Alt+Shift+←/→ reorders draft tokens.',
  typedHelp: 'Shortcuts.txt expands only freshly typed Vietnamese tokens on a typed delimiter, never during IME composition or bulk paste.', snippets: 'F1…F9 snippets', shortcutConflict: 'This key is already used or reserved for an application action.',
  cursor: 'Line / column',
};
export function ft(locale: UiLocale, key: DocumentMessage): string { return locale === 'vi' ? vi[key] : en[key]; }
export function localizedDocumentError(locale: UiLocale, error: AppError): string {
  if (error.code === 'legacyWordUnsupported' || error.code === 'wordImportUnsupported') return ft(locale, 'docUnsupported');
  if (error.code === 'multipleDocumentDrops') return ft(locale, 'multipleDrops');
  if (/[Pp]roject|[Dd]raft|[Rr]tf|[Ll]egacyDocument/u.test(error.code)) return ft(locale, 'invalidProject');
  if (/[Ee]ncoding|[Dd]ecode/u.test(error.code)) return ft(locale, 'invalidEncoding');
  if (/[Rr]ecovery/u.test(error.code)) return ft(locale, 'recoveryFailed');
  if (/[Ss]ettings/u.test(error.code)) return ft(locale, 'settingsError');
  if (/[Dd]ocument|[Ee]xport|[Ss]ave|[Aa]tomic|[Ss]torage|[Ff]ile/u.test(error.code)) return ft(locale, 'fileError');
  return localizedError(locale, error);
}

const VI_IMPORT_WARNINGS: Record<string, string> = {
  'RTF binary payloads were omitted; embedded content is not imported.': 'Đã bỏ dữ liệu nhị phân RTF; không nhập nội dung nhúng.',
  'Nonstandard text in the RTF color table was omitted.': 'Đã bỏ văn bản không chuẩn trong bảng màu RTF.',
  'RTF field instructions were omitted; only displayed field results were imported.': 'Đã bỏ lệnh trường RTF; chỉ nhập văn bản hiển thị.',
  'RTF list labels were imported as plain text; list structure was not imported.': 'Đã nhập nhãn danh sách RTF dưới dạng văn bản; không nhập cấu trúc danh sách.',
  'RTF patterned or double underlines were converted to a single underline.': 'Đã đổi kiểu gạch chân RTF đặc biệt hoặc kép thành gạch chân đơn.',
  'RTF double strikethrough was converted to a single strikethrough.': 'Đã đổi gạch ngang kép RTF thành gạch ngang đơn.',
  'RTF page, column, or section breaks were converted to paragraph breaks; page layout was not imported.': 'Đã đổi ngắt trang, cột hoặc phần RTF thành ngắt đoạn; không nhập bố cục trang.',
  'RTF tables were converted to paragraphs and tabs; table layout was not imported.': 'Đã đổi bảng RTF thành đoạn và tab; không nhập bố cục bảng.',
  'An invalid recovery filename was preserved.': 'Đã giữ nguyên tên tệp khôi phục không hợp lệ.',
};

export function localizedDocumentWarning(locale: UiLocale, warning: string): string {
  if (locale === 'en') return warning;
  const translated = VI_IMPORT_WARNINGS[warning];
  if (translated) return translated;
  let match = /^RTF destination \\(\w+) was omitted; non-text content is not imported\.$/u.exec(warning);
  if (match) return `Đã bỏ vùng RTF \\${match[1]}; không nhập nội dung phi văn bản.`;
  match = /^Unsupported RTF (destination|control symbol) \\(.+) was omitted\.$/u.exec(warning);
  if (match) return `Đã bỏ ${match[1] === 'destination' ? 'vùng' : 'ký hiệu điều khiển'} RTF chưa hỗ trợ \\${match[2]}.`;
  match = /^RTF font charset (\d+) has no supported byte decoder; Unicode text is retained, but its glyph mapping may differ\.$/u.exec(warning);
  if (match) return `Chưa hỗ trợ bảng mã phông RTF ${match[1]}; giữ nguyên văn bản Unicode nhưng hình ký tự có thể khác.`;
  match = /^Legacy scroll index (\d+) was invalid and was reset to 0\.$/u.exec(warning);
  if (match) return `Chỉ số cuộn cũ ${match[1]} không hợp lệ; đã đặt về 0.`;
  match = /^Recovery file (.+) is invalid or unsupported and was preserved for manual recovery\.$/u.exec(warning);
  if (match) return `Tệp khôi phục ${match[1]} không hợp lệ hoặc chưa được hỗ trợ; đã giữ nguyên để khôi phục thủ công.`;
  return `${ft(locale, 'warnings')}: ${warning}`;
}
