import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { LanguageSelector } from "../LanguageSelector";
import { TranslateToEnglish } from "../TranslateToEnglish";
import { useModelStore } from "../../../stores/modelStore";
import type { ModelInfo } from "@/bindings";

export const ModelSettingsCard: React.FC<{ onChooseModel?: () => void }> = ({
  onChooseModel,
}) => {
  const { t } = useTranslation();
  const { currentModel, models } = useModelStore();

  const currentModelInfo = models.find((m: ModelInfo) => m.id === currentModel);

  const showLanguageSelector =
    currentModelInfo?.supports_language_selection ?? false;
  const supportsTranslation = currentModelInfo?.supports_translation ?? false;

  return (
    <SettingsGroup
      title={t("settings.modelSettings.title", {
        model: currentModelInfo?.name ?? t("settings.general.language.noModel"),
      })}
    >
      <LanguageSelector
        descriptionMode="tooltip"
        grouped={true}
        supportedLanguages={currentModelInfo?.supported_languages}
        supportsLanguageDetection={
          currentModelInfo?.supports_language_detection
        }
        supportsLanguageSelection={showLanguageSelector}
        onChooseModel={onChooseModel}
      />
      {supportsTranslation && (
        <TranslateToEnglish descriptionMode="tooltip" grouped={true} />
      )}
    </SettingsGroup>
  );
};
