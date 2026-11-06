import { Platform } from "react-native";

// What this app runs on, for copy like "Unpair from this tablet". Android keeps
// its original wording; iOS names the actual device class.
export const thisDeviceKind = Platform.OS === "ios" ? (Platform.isPad ? "iPad" : "iPhone") : "tablet";

export const isIOS = Platform.OS === "ios";
