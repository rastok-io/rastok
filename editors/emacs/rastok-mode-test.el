;;; rastok-mode-test.el --- Native tests for Rastok coloring -*- lexical-binding: t; -*-

(require 'ert)
(require 'rastok-mode)

(defconst rastok-test--root
  (expand-file-name "../.." (file-name-directory (or load-file-name buffer-file-name))))

(defun rastok-test--face (text face)
  (goto-char (point-min))
  (let ((case-fold-search nil))
    (should (search-forward text nil t)))
  (should (eq face (get-text-property (- (point) (length text)) 'face))))

(defmacro rastok-test--source (source &rest body)
  (declare (indent 1))
  `(with-temp-buffer
     (insert ,source)
     (rastok-mode)
     (font-lock-ensure)
     ,@body))

(ert-deftest rastok-detects-files-and-comments-without-global-settings ()
  (with-temp-buffer
    (setq buffer-file-name "test.rkg")
    (set-auto-mode)
    (should (eq major-mode 'rastok-mode))
    (insert "out = data\n")
    (comment-region (point-min) (point-max))
    (should (equal (buffer-string) "// out = data\n"))
    (uncomment-region (point-min) (point-max))
    (should (equal (buffer-string) "out = data\n"))))

(ert-deftest rastok-colors-declarations-lab-types-and-commands ()
  (rastok-test--source
      (concat "lab smoke { scenario roundtrip { expect 3 app.out == \"hello\" } }\n"
              "simulation alternate : App {}\n"
              "type note = @struct { n: int @measure(id = n) }\n"
              "model.instance.stop\nLoop.again\nRetry.on = [ServerError]\n"
              "connector remote : HttpClient + Retry {}\n"
              "val text = decimal(\"1.25\", 18, 2)\n"
              "input price: decimal(18, 6)\non start { out = 42u }\n")
    (dolist (word '("lab" "scenario" "expect" "simulation" "start"))
      (rastok-test--face word 'font-lock-keyword-face))
    (dolist (word '("smoke" "roundtrip" "text"))
      (rastok-test--face word 'font-lock-variable-name-face))
    (dolist (word '("App" "HttpClient" "@struct" "decimal(18"))
      (rastok-test--face word 'font-lock-type-face))
    (dolist (word '("model.instance.stop" "Loop.again" "Retry.on" "decimal(\""))
      (rastok-test--face word 'font-lock-function-name-face))
    (rastok-test--face "@measure" 'font-lock-preprocessor-face)
    (rastok-test--face "3" 'font-lock-constant-face)
    (rastok-test--face "42u" 'font-lock-constant-face)
    (rastok-test--face "==" 'font-lock-builtin-face)
    (rastok-test--face "hello" 'font-lock-string-face))
  (dolist (type '("struct" "union" "enum" "map" "array" "table" "bool" "int"
                  "long" "uint" "float" "double" "number" "decimal" "text"
                  "bytes" "variant" "date" "time" "uuid" "timestamp"
                  "timestamp_ntz" "duration" "media_type"))
    (rastok-test--source (concat "input request: " type)
      (rastok-test--face type 'font-lock-type-face))))

(ert-deftest rastok-comments-are-not-nested ()
  (dolist (comment '("/* a // not a line comment */" "/* a /* not nested */"))
    (rastok-test--source (concat comment " operator recovered\noperator next")
      (rastok-test--face "not" 'font-lock-comment-face)
      (rastok-test--face "recovered" 'font-lock-variable-name-face)
      (rastok-test--face "next" 'font-lock-variable-name-face))))

(ert-deftest rastok-strings-are-line-bounded ()
  (dolist (quote '("\"" "'"))
    (dolist (suffix '("" "\\"))
      (rastok-test--source (concat "val broken = " quote "unterminated" suffix "\noperator recovered")
	(rastok-test--face "unterminated" 'font-lock-string-face)
	(rastok-test--face "recovered" 'font-lock-variable-name-face)))
    (rastok-test--source (concat "val value = " quote "Loop.again // data" quote "\noperator recovered")
      (rastok-test--face "Loop.again" 'font-lock-string-face)
      (rastok-test--face "recovered" 'font-lock-variable-name-face))))

(ert-deftest rastok-readers-preserve-their-lexical-boundaries ()
  (rastok-test--source
      (concat "val raw = text! {\non in { Loop.done \"{ nested }\" } // raw json!\n}\n"
              "val bytes = base64!(\"text/plain\") { YQ== }\n"
              "val value = json! { \"text\": \"} Loop.again // not DSL\", \"nested\": [true, {\"n\": 2}] }\n"
              "val scalar = json! \"} stop\"\noperator recovered")
    (dolist (word '("text!" "base64!"))
      (rastok-test--face word 'font-lock-function-name-face))
    (rastok-test--face "json!" 'font-lock-string-face)
    (dolist (word '("on in" "Loop.done" "nested }" "// raw" "YQ==" "Loop.again" "stop"))
      (rastok-test--face word 'font-lock-string-face))
    (rastok-test--face "recovered" 'font-lock-variable-name-face)))

(ert-deftest rastok-readers-are-case-sensitive-and-strings-preserve-escapes ()
  (rastok-test--source "val value = \"escaped \\\" quote // text\"\noperator recovered"
    (rastok-test--face "// text" 'font-lock-string-face)
    (rastok-test--face "recovered" 'font-lock-variable-name-face))
  (rastok-test--source "val raw = Text! {\nLoop.again\n}"
    (rastok-test--face "Loop.again" 'font-lock-function-name-face))
  (rastok-test--source "val raw = text! {\nLoop.again // payload"
    (rastok-test--face "Loop.again" 'font-lock-string-face)))

(ert-deftest rastok-editing-a-reader-removes-stale-syntax ()
  (rastok-test--source "val raw = text! {\non in { Loop.done }\n}\noperator recovered"
    (rastok-test--face "Loop.done" 'font-lock-string-face)
    (goto-char (point-min))
    (search-forward "text! ")
    (delete-region (- (point) 6) (point))
    (font-lock-ensure)
    (rastok-test--face "Loop.done" 'font-lock-function-name-face)
    (goto-char (point-min))
    (search-forward "= ")
    (insert "text! ")
    (font-lock-ensure)
    (rastok-test--face "Loop.done" 'font-lock-string-face)
    (rastok-test--face "recovered" 'font-lock-variable-name-face)))

(ert-deftest rastok-all-shipped-examples-return-to-host-syntax ()
  (let ((examples (directory-files (expand-file-name "examples" rastok-test--root) t "\\.rkg\\'")))
    (should examples)
    (dolist (file examples)
      (rastok-test--source (with-temp-buffer
                             (insert-file-contents file)
                             (concat (buffer-string) "\noperator tokenizer_sentinel"))
	(rastok-test--face "tokenizer_sentinel" 'font-lock-variable-name-face)))))

;;; rastok-mode-test.el ends here
