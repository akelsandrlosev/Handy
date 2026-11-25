import React from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { SettingContainer } from "../ui/SettingContainer";
import { Button } from "../ui/Button";
import { useSettings } from "../../hooks/useSettings";

interface BugReportFolderProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const BugReportFolder: React.FC<BugReportFolderProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const folder = getSetting("bug_report_dir") || null;

    const choose = async () => {
      const picked = await open({
        directory: true,
        defaultPath: folder ?? undefined,
      });
      if (typeof picked === "string") {
        await updateSetting("bug_report_dir", picked);
      }
    };

    return (
      <SettingContainer
        title={t("settings.bugReport.folder.title")}
        description={t("settings.bugReport.folder.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        layout="stacked"
      >
        <div className="flex items-center gap-2">
          <span
            className="flex-1 truncate text-sm text-text/70"
            title={folder ?? ""}
          >
            {folder ?? t("settings.bugReport.folder.default")}
          </span>
          <Button
            variant="secondary"
            size="sm"
            onClick={choose}
            disabled={isUpdating("bug_report_dir")}
          >
            {t("settings.bugReport.folder.choose")}
          </Button>
          {folder && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => updateSetting("bug_report_dir", null)}
              disabled={isUpdating("bug_report_dir")}
            >
              {t("settings.bugReport.folder.reset")}
            </Button>
          )}
        </div>
      </SettingContainer>
    );
  },
);
