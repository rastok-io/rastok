;;; rastok-mode.el --- Highlight Rastok graph sources -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: Apache-2.0
;; Package-Requires: ((emacs "27.1"))

;;; Commentary:
;; Source coloring and comment commands only.  Use `rastok check' for validation.
;; Raw text/base64 readers count braces even inside quotes; ordinary strings
;; end at a physical line.  Neither rule is the default C-like syntax table.

;;; Code:

(defvar rastok-mode-syntax-table
  (let ((table (make-syntax-table)))
    (modify-syntax-entry ?_ "w" table)
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    (modify-syntax-entry ?\r "> b" table)
    (modify-syntax-entry ?\" "\"" table)
    (modify-syntax-entry ?\' "\"" table)
    (modify-syntax-entry ?\\ "\\" table)
    table)
  "Syntax table for Rastok comments, identifiers and strings.")

(defconst rastok--raw-start
  (concat "\\_<\\(?:text\\|base64\\)!"
          "\\(?:[ \t]*(\\s-*\"\\(?:\\\\.\\|[^\"\\\\]\\)*\"\\s-*)\\)?"
          "[ \t\r\n]*{"))

(defun rastok--syntax-propertize (_start _end)
  "Mark raw reader bodies and line-bounded strings in the current buffer.
Recompute the buffer so an edited opener cannot leave stale reader state."
  (save-excursion
    (save-restriction
      (widen)
      (let ((case-fold-search nil))
	(remove-text-properties (point-min) (point-max) '(syntax-table nil))
	(goto-char (point-min))
	(while (re-search-forward
		(concat "\\(" rastok--raw-start "\\)\\|//\\|/\\*\\|[\"']") nil t)
          (cond
           ((match-beginning 1)
            (let ((open (1- (point))) (depth 1))
              (while (and (> depth 0) (re-search-forward "[{}]" nil t))
		(setq depth (+ depth (if (eq (char-before) ?{) 1 -1))))
              (unless (zerop depth) (goto-char (point-max)))
              ;; In a verbatim payload quotes, slashes and escapes are all data.
              (put-text-property open (point) 'syntax-table (string-to-syntax "."))
              (put-text-property open (1+ open) 'syntax-table (string-to-syntax "|"))
              (when (zerop depth)
		(put-text-property (1- (point)) (point) 'syntax-table
                                   (string-to-syntax "|")))))
           ((equal (match-string 0) "//") (end-of-line))
           ((equal (match-string 0) "/*")
            (unless (search-forward "*/" nil t) (goto-char (point-max))))
           (t
            (let ((quote (char-before)) (finished nil))
              ;; Generic fences also let a physical newline close either quote.
              (put-text-property (1- (point)) (point) 'syntax-table
				 (string-to-syntax "|"))
              (while (and (not finished) (< (point) (point-max)))
		(let ((char (char-after)))
                  (cond
                   ((eq char quote)
                    (put-text-property (point) (1+ (point)) 'syntax-table
                                       (string-to-syntax "|"))
                    (forward-char)
                    (setq finished t))
                   ((memq char '(?\n ?\r))
                    (put-text-property (point) (1+ (point)) 'syntax-table
                                       (string-to-syntax "|"))
                    (forward-char)
                    (setq finished t))
                   ((eq char ?\\)
                    (when (memq (char-after (1+ (point))) '(?\n ?\r))
                      (put-text-property (point) (1+ (point)) 'syntax-table
					 (string-to-syntax ".")))
                    (forward-char)
                    (when (and (< (point) (point-max))
                               (not (memq (char-after) '(?\n ?\r))))
                      (forward-char)))
                   (t (forward-char)))))))))))))

(defun rastok--syntax-region (_start _end)
  "Extend syntax processing to include any edited reader or string opener."
  (cons (point-min) (point-max)))

(defconst rastok--font-lock-keywords
  (let ((declarations '("graph" "operator" "connector" "lab" "simulation"
                        "experiment" "theorem" "scenario" "mock" "type"
                        "input" "output" "val" "var"))
        (types '("struct" "union" "enum" "map" "array" "table" "bool" "int"
                 "long" "uint" "float" "double" "number" "decimal" "text"
                 "bytes" "variant" "date" "time" "uuid" "timestamp"
                 "timestamp_ntz" "duration" "media_type"))
        (keywords '("on" "start" "where" "within" "last" "match" "nothing"
                    "stop" "expect" "advance" "crash" "after" "restart"
                    "origin" "is" "as" "in")))
    `((,(concat (regexp-opt declarations 'symbols)
                "[ \t]+\\([A-Za-z_][A-Za-z0-9_]*\\)")
       (1 font-lock-keyword-face) (2 font-lock-variable-name-face))
      ("@struct\\_>" . font-lock-type-face)
      ("@[A-Za-z_][A-Za-z0-9_]*" . font-lock-preprocessor-face)
      ("\\_<\\(?:text\\|base64\\|json\\)!" . font-lock-function-name-face)
      ("\\_<Loop\\.\\(?:next\\|again\\|done\\|state\\|fuel\\|delay\\|measure\\|entry\\|terminal\\|expired\\)\\_>" . font-lock-function-name-face)
      ("\\_<Retry\\.\\(?:attempts\\|delay\\|on\\)\\_>" . font-lock-function-name-face)
      ("\\_<\\(?:[A-Za-z_][A-Za-z0-9_]*\\.\\)*instance\\.stop\\_>" . font-lock-function-name-face)
      (,(regexp-opt '("App" "Loop" "Retry" "Saga" "Process" "File"
                      "HttpClient" "HttpServer" "WebSocketClient" "WebSocketServer") 'symbols)
       . font-lock-type-face)
      (,(regexp-opt keywords 'symbols) . font-lock-keyword-face)
      ("\\(\\_<decimal\\_>\\)[ \t]*(\\s-*[0-9]+[ \t]*," (1 font-lock-type-face))
      (,(concat "\\(" (regexp-opt '("env" "secret" "asset" "duration" "timestamp"
                                    "decimal" "media_type" "size" "has" "now") 'symbols)
                "\\)[ \t]*(") (1 font-lock-function-name-face))
      (,(regexp-opt types 'symbols) . font-lock-type-face)
      (,(regexp-opt '("true" "false" "null") 'symbols) . font-lock-constant-face)
      (,(regexp-opt '("data" "meta" "instance" "lifetime" "closing" "behavior"
                      "idempotent" "json_profile") 'symbols) . font-lock-variable-name-face)
      ("\\_<[0-9]+\\(?:u\\|\\.[0-9]+\\)?\\_>" . font-lock-constant-face)
      ("<->\\|->\\|=>\\|==\\|!=\\|<=\\|>=\\|&&\\|||\\|[=+*/%!?<>|.-]" . font-lock-builtin-face)))
  "Font Lock rules for declarations, types, expressions and Lab steps.")

;;;###autoload
(define-derived-mode rastok-mode prog-mode "Rastok"
  "Major mode for coloring Rastok .rkg sources.
This mode does not provide completion, semantic navigation or type checking."
  :syntax-table rastok-mode-syntax-table
  (setq-local font-lock-defaults '(rastok--font-lock-keywords))
  (setq-local syntax-propertize-function #'rastok--syntax-propertize)
  (add-hook 'syntax-propertize-extend-region-functions #'rastok--syntax-region nil t)
  (setq-local comment-start "// ")
  (setq-local comment-end "")
  (setq-local comment-start-skip "\\(?://+\\|/\\*+\\)[ \t]*"))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.rkg\\'" . rastok-mode))

(provide 'rastok-mode)
;;; rastok-mode.el ends here
