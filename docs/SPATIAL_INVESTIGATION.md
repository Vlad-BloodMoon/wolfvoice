# Spatial listener-axis investigation

Reported: local voice becomes quiet when following a moving speaker; moving the camera can restore audibility even with avatar-position listening selected.

Confirmed source defect: src/mixer.rs treats local +X as listener-right, while Firestorm's LLCoordFrame defines +X forward and +Y left. A source directly in front is therefore routed hard right in the released mixer. The test reproduces that failure on the released code before checking the corrected code.

Candidate change: use local -Y for right, normalize the quantized orientation quaternion, and use full 3D direction length for elevation-aware panning. Tests cover front/back, left/right, yaw and pitch, wire rounding, power around an orbit, common translation, elevation, and invalid/large quaternions.

This does NOT establish the cause of every reported dropout. Both old and new panning retain total stereo power. A receiver dropping one stereo channel, actual listener coordinates, attenuation, room/parcel transitions and transport need live evidence if silence persists. Firestorm avatar mode should use avatar position AND orientation; mixed mode uses avatar position and camera orientation.

The currently deployed grid uses a credential-only setup without RpcKeyFile; the region-fed position feature returns early when no relay key is available, so that feed is not active in the documented deployment. Adding a relay key on the shared server would affect unchanged connected simulators and is not part of this investigation.

No stable release or live service is changed. Do not claim the reported walking/camera dropout fixed until a viewer test reproduces it and passes with the candidate build.

Sources:
- Firestorm indra/llmath/llcoordframe.h getAtAxis/getLeftAxis/getUpAxis.
- Firestorm indra/newview/llvoicewebrtc.cpp updatePosition/sendPositionUpdate.
- Firestorm indra/newview/llvoicewebrtc.h earLocCamera/earLocAvatar/earLocMixed.
