import React, { useState, useRef, useEffect, useMemo, useId } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { ShortcutInput } from "./ShortcutInput";
import { useModelStore } from "../../stores/modelStore";
import {
  commands,
  type RecognitionLanguage,
  type RecognitionLanguageStatus,
  type RecognitionLanguageError,
  type RecognitionMode,
} from "../../bindings";
import {
  getLanguageLabel,
  LANGUAGES,
  type Language,
} from "../../lib/constants/languages";
import type { ModelStateEvent } from "../../lib/types/events";

interface LanguageSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  supportedLanguages?: string[];
  supportsLanguageDetection?: boolean;
  supportsLanguageSelection?: boolean;
  onChooseModel?: () => void;
}

const DEFAULT_RECOGNITION_LANGUAGE: RecognitionLanguage = {
  mode: "auto",
  single_language: "",
  pair: [],
};

const controlClass =
  "px-2 py-1 text-sm font-semibold bg-mid-gray/10 border border-mid-gray/80 rounded min-w-0 max-w-full text-start hover:bg-logo-primary/10 hover:border-logo-primary disabled:opacity-50 disabled:cursor-not-allowed";

// Keep the existing searchable language presentation, including raw custom-model
// codes. Resolution and duplicate detection belong exclusively to the backend.
const LanguagePicker: React.FC<{
  label: string;
  value: string;
  options: Language[];
  disabled: boolean;
  onSelect: (value: string) => void;
  selector: string;
}> = ({ label, value, options, disabled, onSelect, selector }) => {
  const { t } = useTranslation();
  const id = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const filtered = options.filter((option) =>
    `${option.label} ${option.value}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );
  const close = () => {
    setOpen(false);
    setQuery("");
    setActive(0);
  };
  const select = (code: string) => {
    onSelect(code);
    close();
    triggerRef.current?.focus();
  };
  useEffect(() => {
    const outside = (event: MouseEvent) => {
      if (!wrapperRef.current?.contains(event.target as Node)) {
        setOpen(false);
        setQuery("");
        setActive(0);
      }
    };
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, []);
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);
  useEffect(() => {
    if (open) {
      document
        .getElementById(`${id}-option-${active}`)
        ?.scrollIntoView({ block: "nearest" });
    }
  }, [active, open, query, id]);
  return (
    <div
      className="relative"
      ref={wrapperRef}
      data-recognition-picker={selector}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          close();
          triggerRef.current?.focus();
        }
      }}
    >
      <span id={`${id}-label`} className="block text-sm mb-1">
        {label}
      </span>
      <button
        ref={triggerRef}
        type="button"
        aria-labelledby={`${id}-label ${id}-value`}
        aria-haspopup="listbox"
        aria-expanded={open && !disabled}
        aria-controls={`${id}-options`}
        className={`${controlClass} w-full flex items-center justify-between`}
        disabled={disabled}
        onClick={() => (open ? close() : setOpen(true))}
      >
        <span id={`${id}-value`} className="truncate">
          {options.find((option) => option.value === value)?.label ||
            getLanguageLabel(value) ||
            value ||
            t("settings.general.language.chooseLanguage")}
        </span>
        <svg
          aria-hidden="true"
          className="w-4 h-4 ms-2 shrink-0"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>
      {open && !disabled && (
        <div className="absolute top-full start-0 w-full mt-1 bg-background border border-mid-gray/80 rounded shadow-lg z-50 overflow-hidden">
          <div className="p-2 border-b border-mid-gray/80">
            <input
              ref={inputRef}
              role="combobox"
              aria-label={t("settings.general.language.searchPlaceholder")}
              aria-expanded="true"
              aria-controls={`${id}-options`}
              aria-autocomplete="list"
              aria-activedescendant={
                filtered[active] ? `${id}-option-${active}` : undefined
              }
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setActive(0);
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  close();
                  triggerRef.current?.focus();
                } else if (
                  event.key === "ArrowDown" ||
                  event.key === "ArrowUp"
                ) {
                  event.preventDefault();
                  setActive((index) =>
                    Math.max(
                      0,
                      Math.min(
                        filtered.length - 1,
                        index + (event.key === "ArrowDown" ? 1 : -1),
                      ),
                    ),
                  );
                } else if (event.key === "Enter" && filtered[active]) {
                  event.preventDefault();
                  select(filtered[active].value);
                }
              }}
              placeholder={t("settings.general.language.searchPlaceholder")}
              className="w-full px-2 py-1 text-sm bg-mid-gray/10 border border-mid-gray/40 rounded focus:outline-none focus:ring-1 focus:ring-logo-primary"
            />
          </div>
          <div
            id={`${id}-options`}
            role="listbox"
            aria-labelledby={`${id}-label`}
            className="max-h-48 overflow-y-auto"
          >
            {filtered.map((option, index) => (
              <button
                id={`${id}-option-${index}`}
                key={option.value}
                type="button"
                role="option"
                aria-selected={value === option.value}
                className={`w-full px-2 py-1 text-sm text-start hover:bg-logo-primary/10 ${active === index ? "bg-logo-primary/10" : ""} ${value === option.value ? "text-logo-primary font-semibold" : ""}`}
                onClick={() => select(option.value)}
              >
                {option.label}
              </button>
            ))}
            {filtered.length === 0 && (
              <p className="px-2 py-2 text-sm text-mid-gray">
                {t("settings.general.language.noResults")}
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export const LanguageSelector: React.FC<LanguageSelectorProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  supportedLanguages,
  supportsLanguageDetection = true,
  supportsLanguageSelection = true,
  onChooseModel,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating, isLoading } = useSettings();
  const currentModel = useModelStore((state) => state.currentModel);
  const savedIntent =
    getSetting("recognition_language") ?? DEFAULT_RECOGNITION_LANGUAGE;
  const intent = useMemo(
    () => ({
      ...savedIntent,
      single_language: savedIntent.single_language ?? "",
      pair: savedIntent.pair ?? [],
    }),
    [savedIntent],
  );
  const updating = isUpdating("recognition_language");
  const [status, setStatus] = useState<RecognitionLanguageStatus | null>(null);
  const [revision, setRevision] = useState(0);
  const mode = intent.mode;
  const disabled = updating || isLoading;

  useEffect(() => {
    const refresh = () => setRevision((value) => value + 1);
    const listeners = [
      listen<ModelStateEvent>("model-state-changed", (event) => {
        if (event.payload.event_type === "loading_started") {
          setStatus(null);
        } else {
          refresh();
        }
      }),
      listen("models-updated", refresh),
      listen("settings-changed", refresh),
    ];
    return () => {
      listeners.forEach((listener) => {
        void listener.then((unlisten) => unlisten());
      });
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setStatus(null);
    if (!disabled) {
      commands
        .getRecognitionLanguageStatus(null)
        .then((result) => {
          if (!cancelled) setStatus(result);
        })
        .catch(() => {
          if (!cancelled) setStatus({ status: "unknown" });
        });
    }
    return () => {
      cancelled = true;
    };
  }, [
    intent,
    currentModel,
    supportedLanguages,
    supportsLanguageDetection,
    disabled,
    revision,
  ]);

  const availableLanguages = useMemo(() => {
    const codes =
      supportedLanguages ??
      LANGUAGES.filter((language) => language.value !== "auto").map(
        (language) => language.value,
      );
    return [...new Set(codes)]
      .filter((code) => code !== "auto")
      .map((code) => ({
        value: code,
        label: getLanguageLabel(code) || code,
      }));
  }, [supportedLanguages]);
  const optionsFor = (value: string) =>
    value && !availableLanguages.some((option) => option.value === value)
      ? [
          ...availableLanguages,
          {
            value,
            label: t("settings.general.language.savedChoice", {
              language: getLanguageLabel(value) || value,
            }),
          },
        ]
      : availableLanguages;
  const reasonText = (reason: RecognitionLanguageError) =>
    reason.code === "unsupported_candidate"
      ? t("settings.general.language.reasons.unsupported_candidate", {
          language: getLanguageLabel(reason.language) || reason.language,
        })
      : reason.code === "model_unavailable"
        ? t("settings.general.language.reasons.model_unavailable", {
            message: reason.message,
          })
        : t(`settings.general.language.reasons.${reason.code}`);
  const resolution = status?.status === "ready" ? status.resolution : null;
  const setMode = async (nextMode: RecognitionMode) => {
    const nextIntent = { ...intent, mode: nextMode };
    if (
      nextMode === "single" &&
      (!nextIntent.single_language || nextIntent.single_language === "auto")
    ) {
      const language =
        availableLanguages.find((option) => option.value === "en")?.value ??
        availableLanguages[0]?.value;
      if (!language) return;
      nextIntent.single_language = language;
    }
    await updateSetting("recognition_language", nextIntent);
  };

  return (
    <SettingContainer
      title={t("settings.general.language.title")}
      description={t("settings.general.language.recognitionDescription")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="stacked"
    >
      <div className="space-y-3" data-recognition-settings>
        <fieldset disabled={disabled} className="flex flex-wrap gap-3">
          <legend className="sr-only">
            {t("settings.general.language.mode")}
          </legend>
          {(["auto", "single", "pair"] as const).map((option) => (
            <label
              key={option}
              className="flex items-center gap-2 text-sm cursor-pointer"
            >
              <input
                type="radio"
                name="recognition-mode"
                value={option}
                checked={mode === option}
                disabled={
                  disabled ||
                  (option === "auto" &&
                    supportsLanguageSelection &&
                    !supportsLanguageDetection &&
                    intent.mode !== "auto") ||
                  (option === "single" &&
                    ((!supportsLanguageSelection && intent.mode !== "single") ||
                      (availableLanguages.length === 0 &&
                        (!intent.single_language ||
                          intent.single_language === "auto"))))
                }
                onChange={() => {
                  void setMode(option);
                }}
                className="accent-logo-primary"
              />
              {t(`settings.general.language.modes.${option}`)}
            </label>
          ))}
        </fieldset>
        {mode === "single" && supportsLanguageSelection && (
          <LanguagePicker
            label={t("settings.general.language.modes.single")}
            value={
              resolution?.mode === "single"
                ? resolution.language
                : intent.single_language
            }
            options={optionsFor(intent.single_language)}
            disabled={disabled}
            onSelect={(language) => {
              void updateSetting("recognition_language", {
                ...intent,
                mode: "single",
                single_language: language,
              });
            }}
            selector="single"
          />
        )}
        {!supportsLanguageSelection && mode !== "pair" && (
          <p className="text-sm text-mid-gray">
            {t("settings.general.language.modelManaged")}
          </p>
        )}
        {mode === "pair" && (
          <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,13rem),1fr))] gap-3">
            {[0, 1].map((index) => (
              <div key={index} className="min-w-0 space-y-2">
                <div className="px-4 pt-2">
                  <LanguagePicker
                    label={t("settings.general.language.pairLanguage", {
                      number: index + 1,
                    })}
                    value={intent.pair[index] ?? ""}
                    options={optionsFor(intent.pair[index] ?? "")}
                    disabled={disabled}
                    onSelect={(language) => {
                      void updateSetting("recognition_language", {
                        ...intent,
                        pair:
                          index === 0
                            ? [language, intent.pair[1] ?? ""]
                            : [intent.pair[0] ?? "", language],
                      });
                    }}
                    selector={`candidate-${index + 1}`}
                  />
                </div>
                <ShortcutInput
                  shortcutId={`transcribe_language_${index + 1}`}
                  allowClear
                  disabled={disabled}
                  grouped
                />
              </div>
            ))}
          </div>
        )}
        <div
          role="status"
          aria-live="polite"
          data-recognition-status
          className="text-sm"
        >
          {status?.status === "incompatible" ? (
            <p className="text-red-500">
              {t("settings.general.language.blocked", {
                reason: reasonText(status.reason),
              })}
            </p>
          ) : intent.mode === "pair" &&
            (!status || status.status === "unknown") ? (
            <p>{t("settings.general.language.pending")}</p>
          ) : intent.mode === "pair" && resolution?.mode === "pair" ? (
            <p>
              {t("settings.general.language.readyPair", {
                languages: resolution.languages
                  .map((code) => getLanguageLabel(code) || code)
                  .join(" / "),
              })}
            </p>
          ) : null}
        </div>
        {intent.mode === "pair" && status?.status !== "ready" && (
          <div className="space-y-2">
            <p className="text-sm text-mid-gray">
              {t("settings.general.language.recovery")}
            </p>
            {onChooseModel && (
              <button
                type="button"
                data-recognition-action="choose-model"
                className={controlClass}
                onClick={onChooseModel}
              >
                {t("settings.general.language.chooseModel")}
              </button>
            )}
          </div>
        )}
      </div>
    </SettingContainer>
  );
};
