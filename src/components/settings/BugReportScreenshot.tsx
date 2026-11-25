import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface BugReportScreenshotProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const BugReportScreenshot: React.FC<BugReportScreenshotProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("bug_report_screenshot") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("bug_report_screenshot", value)}
        isUpdating={isUpdating("bug_report_screenshot")}
        label={t("settings.bugReport.screenshot.label")}
        description={t("settings.bugReport.screenshot.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
