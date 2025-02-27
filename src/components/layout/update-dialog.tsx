import useSWR from "swr";
import { useState } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
} from "@mui/material";
import { relaunch } from "@tauri-apps/api/process";
import { checkUpdate, installUpdate } from "@tauri-apps/api/updater";
import { killSidecars, restartSidecar } from "../../services/cmds";
import Notice from "../base/base-notice";

interface Props {
  open: boolean;
  onClose: () => void;
}

let uploadingState = false;

const UpdateDialog = (props: Props) => {
  const { open, onClose } = props;
  const { data: updateInfo } = useSWR("checkUpdate", checkUpdate, {
    errorRetryCount: 2,
    revalidateIfStale: false,
    focusThrottleInterval: 36e5, // 1 hour
  });
  const [uploading, setUploading] = useState(uploadingState);

  const onUpdate = async () => {
    setUploading(true);
    uploadingState = true;

    try {
      await killSidecars();
      await installUpdate();
      await relaunch();
    } catch (err: any) {
      await restartSidecar();
      Notice.error(err?.message || err.toString());
    } finally {
      setUploading(false);
      uploadingState = false;
    }
  };

  return (
    <Dialog open={open} onClose={onClose}>
      <DialogTitle>New Version v{updateInfo?.manifest?.version}</DialogTitle>
      <DialogContent sx={{ minWidth: 360, maxWidth: 400, maxHeight: "50vh" }}>
        <DialogContentText>{updateInfo?.manifest?.body}</DialogContentText>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button
          variant="contained"
          autoFocus
          onClick={onUpdate}
          disabled={uploading}
        >
          Update
        </Button>
      </DialogActions>
    </Dialog>
  );
};

export default UpdateDialog;
