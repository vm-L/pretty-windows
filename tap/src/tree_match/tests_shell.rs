use super::*;

#[test]
fn jumplist_scope_targets_taskbar_jumplist_and_excludes_notifications_and_items() {
    let mut idx = VisualTreeIndex::new();

    // 1. Win11 TaskbarJumpListFrame > Grid#JumpListGrid > Border#JumpListRestyledAcrylic + item chrome
    idx.on_add_on_thread(
        0,
        4001,
        "JumpViewUI.TaskbarJumpListFrame".into(),
        "TaskbarJumpListFrame".into(),
        200,
    );
    idx.on_add_on_thread(
        4001,
        4002,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "JumpListGrid".into(),
        200,
    );
    idx.on_add_on_thread(
        4002,
        4003,
        "Windows.UI.Xaml.Controls.Border".into(),
        "JumpListRestyledAcrylic".into(),
        200,
    );
    idx.on_add_on_thread(
        4002,
        4004,
        "JumpViewUI.JumpListItemControl".into(),
        "".into(),
        200,
    );
    idx.on_add_on_thread(
        4004,
        4005,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        200,
    );
    idx.on_add_on_thread(
        4002,
        4006,
        "JumpViewUI.SystemItemControl".into(),
        "".into(),
        200,
    );
    idx.on_add_on_thread(
        4006,
        4007,
        "Windows.UI.Xaml.Controls.Border".into(),
        "LayoutRoot".into(),
        200,
    );

    // 2. Nested context menu ControlHostMenuFlyoutPresenter inside JumpViewUI
    idx.on_add_on_thread(
        0,
        4010,
        "JumpViewUI.ControlHostMenuFlyoutPresenter".into(),
        "".into(),
        200,
    );
    idx.on_add_on_thread(
        4010,
        4011,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        200,
    );

    // 3. Notifications / Action Center Toast and nested MenuFlyoutPresenter must be excluded
    idx.on_add_on_thread(
        0,
        4020,
        "ActionCenter.ToastNotificationView".into(),
        "ToastRoot".into(),
        200,
    );
    idx.on_add_on_thread(
        4020,
        4021,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        200,
    );
    idx.on_add_on_thread(
        4021,
        4022,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        200,
    );
    // Plain MenuFlyoutPresenter in ShellExperienceHost without JumpList ancestry must not match
    idx.on_add_on_thread(
        0,
        4025,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        200,
    );
    idx.on_add_on_thread(
        4025,
        4026,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        200,
    );

    assert_eq!(
        idx.classify_target_in_scope(4003, TapHostScope::ShellExperience),
        Some(TargetKind::MenuFlyoutSurface)
    );
    // When JumpListRestyledAcrylic is present on the thread, Grid#JumpListGrid must not duplicate it
    assert_eq!(
        idx.classify_target_in_scope(4002, TapHostScope::ShellExperience),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(4011, TapHostScope::ShellExperience),
        Some(TargetKind::MenuFlyoutSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(4005, TapHostScope::ShellExperience),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(4007, TapHostScope::ShellExperience),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(4022, TapHostScope::ShellExperience),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(4026, TapHostScope::ShellExperience),
        None
    );

    // 4. Legacy/variant thread without JumpListRestyledAcrylic falls back to TaskbarJumpListFrame > Border#LayoutRoot
    idx.on_add_on_thread(
        0,
        4030,
        "JumpViewUI.TaskbarJumpListFrame".into(),
        "".into(),
        201,
    );
    idx.on_add_on_thread(
        4030,
        4031,
        "Windows.UI.Xaml.Controls.Border".into(),
        "LayoutRoot".into(),
        201,
    );
    assert_eq!(
        idx.classify_target_in_scope(4031, TapHostScope::ShellExperience),
        Some(TargetKind::MenuFlyoutSurface)
    );
}

#[test]
fn shell_experience_targets_notifications_calendar_toasts_and_isolates_history() {
    let mut idx = VisualTreeIndex::new();

    // 1. Notification Center + Calendar + minimized header overlay + history toast item
    idx.on_add_on_thread(
        0,
        5001,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "NotificationCenterGrid".into(),
        301,
    );
    idx.on_add_on_thread(
        5001,
        5002,
        "ActionCenter.FlexibleNormalToastView".into(),
        "".into(),
        301,
    );
    idx.on_add_on_thread(
        5002,
        5003,
        "Windows.UI.Xaml.Controls.Border".into(),
        "ToastBackgroundBorder2".into(),
        301,
    );
    idx.on_add_on_thread(
        5001,
        5004,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "FooterGrid".into(),
        301,
    );
    idx.on_add_on_thread(
        0,
        5010,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "CalendarCenterGrid".into(),
        301,
    );
    idx.on_add_on_thread(
        5010,
        5011,
        "Windows.UI.Xaml.Controls.Border".into(),
        "CalendarHeaderMinimizedOverlay".into(),
        301,
    );
    idx.on_add_on_thread(
        5010,
        5012,
        "Windows.UI.Xaml.Controls.Primitives.CalendarViewDayItem".into(),
        "".into(),
        301,
    );
    idx.on_add_on_thread(
        5012,
        5013,
        "Windows.UI.Xaml.Controls.Border".into(),
        "ToastBackgroundBorder2".into(),
        301,
    );

    assert_eq!(
        idx.classify_target_in_scope(5001, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5003, TapHostScope::ShellExperience),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(5004, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5010, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5011, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5013, TapHostScope::ShellExperience),
        None
    );

    // 2. Floating Toast with both ToastBackgroundBorder2 and inner ToastBackgroundBorder + interactive Button
    idx.on_add_on_thread(
        0,
        5100,
        "ActionCenter.FlexibleNormalToastView".into(),
        "".into(),
        302,
    );
    idx.on_add_on_thread(
        5100,
        5101,
        "Windows.UI.Xaml.Controls.Border".into(),
        "ToastBackgroundBorder2".into(),
        302,
    );
    idx.on_add_on_thread(
        5101,
        5102,
        "Windows.UI.Xaml.Controls.Border".into(),
        "ToastBackgroundBorder".into(),
        302,
    );
    idx.on_add_on_thread(
        5101,
        5103,
        "Windows.UI.Xaml.Controls.Button".into(),
        "DismissButton".into(),
        302,
    );
    idx.on_add_on_thread(
        5103,
        5104,
        "Windows.UI.Xaml.Controls.Border".into(),
        "ToastBackgroundBorder".into(),
        302,
    );

    assert_eq!(
        idx.classify_target_in_scope(5101, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5102, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5104, TapHostScope::ShellExperience),
        None
    );

    // 3. Single-border floating toast without ToastBackgroundBorder2 promotes ToastBackgroundBorder
    idx.on_add_on_thread(
        0,
        5200,
        "ActionCenter.FlexiblePriorityToastView".into(),
        "".into(),
        303,
    );
    idx.on_add_on_thread(
        5200,
        5201,
        "Windows.UI.Xaml.Controls.Border".into(),
        "ToastBackgroundBorder".into(),
        303,
    );
    assert_eq!(
        idx.classify_target_in_scope(5201, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSurface)
    );

    // 4. Legacy Win11 ActionCenter Quick Settings + Media Card inside ShellExperienceHost
    idx.on_add_on_thread(
        0,
        5300,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ControlCenterRegion".into(),
        304,
    );
    idx.on_add_on_thread(
        0,
        5301,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "MediaTransportControlsRegion".into(),
        304,
    );
    idx.on_add_on_thread(
        5300,
        5302,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "FooterGrid".into(),
        304,
    );
    assert_eq!(
        idx.classify_target_in_scope(5300, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5301, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(5302, TapHostScope::ShellExperience),
        Some(TargetKind::ShellCardSubSurface)
    );
}

#[test]
fn shell_host_targets_quick_settings_media_card_subsurfaces_and_fallback_order() {
    let mut idx = VisualTreeIndex::new();

    // 1. Thread without RootGridBorder: MainGrid + L1Grid delivered before RootGridBorder
    idx.on_add_on_thread(0, 6000, "ControlCenter.ControlCenterView".into(), "".into(), 401);
    idx.on_add_on_thread(6000, 6001, "Windows.UI.Xaml.Controls.Grid".into(), "RootGrid".into(), 401);
    idx.on_add_on_thread(6001, 6002, "Windows.UI.Xaml.Controls.Grid".into(), "MainGrid".into(), 401);
    idx.on_add_on_thread(6002, 6003, "Windows.UI.Xaml.Controls.Grid".into(), "L1Grid".into(), 401);
    idx.on_add_on_thread(6003, 6004, "Windows.UI.Xaml.Controls.Border".into(), "BackgroundBlur".into(), 401);
    idx.on_add_on_thread(6003, 6005, "Windows.UI.Xaml.Controls.Grid".into(), "PaneFooter".into(), 401);
    idx.on_add_on_thread(
        6001,
        6006,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "MediaTransportControlsRegion".into(),
        401,
    );
    // Interactive quick toggle tile border must stay untouched
    idx.on_add_on_thread(
        6003,
        6007,
        "Windows.UI.Xaml.Controls.Primitives.ToggleButton".into(),
        "".into(),
        401,
    );
    idx.on_add_on_thread(
        6007,
        6008,
        "Windows.UI.Xaml.Controls.Border".into(),
        "RootGridBorder".into(),
        401,
    );

    assert_eq!(
        idx.classify_target_in_scope(6002, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(6003, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(6004, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(6005, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(6006, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(6008, TapHostScope::ShellHost),
        None
    );

    // Once non-button RootGridBorder arrives on the same thread, L1Grid downgrades to SubSurface
    idx.on_add_on_thread(
        6002,
        6009,
        "Windows.UI.Xaml.Controls.Border".into(),
        "RootGridBorder".into(),
        401,
    );
    assert_eq!(
        idx.classify_target_in_scope(6009, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(6003, TapHostScope::ShellHost),
        Some(TargetKind::ShellCardSubSurface)
    );

    // 2. Context menu inside ShellHost matches MenuFlyoutSurface
    idx.on_add_on_thread(
        0,
        6020,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        401,
    );
    idx.on_add_on_thread(
        6020,
        6021,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        401,
    );
    assert_eq!(
        idx.classify_target_in_scope(6021, TapHostScope::ShellHost),
        Some(TargetKind::MenuFlyoutSurface)
    );
}

#[test]
fn start_menu_targets_acrylic_border_overlay_shadows_and_context_menus() {
    let mut idx = VisualTreeIndex::new();

    // 1. Win11 DockedStart: FrameRoot > MaxHeightEnforcer > StartDropShadow + MainMenu > AcrylicBorder + MainContent > AcrylicOverlay
    idx.on_add_on_thread(
        0,
        9000,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "FrameRoot".into(),
        701,
    );
    idx.on_add_on_thread(
        9000,
        9001,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "MaxHeightEnforcer".into(),
        701,
    );
    idx.on_add_on_thread(
        9001,
        9002,
        "Windows.UI.Xaml.Controls.Border".into(),
        "StartDropShadow".into(),
        701,
    );
    idx.on_add_on_thread(
        9001,
        9003,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "MainMenu".into(),
        701,
    );
    idx.on_add_on_thread(
        9003,
        9004,
        "Windows.UI.Xaml.Controls.Border".into(),
        "AcrylicBorder".into(),
        701,
    );
    idx.on_add_on_thread(
        9003,
        9005,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "MainContent".into(),
        701,
    );
    idx.on_add_on_thread(
        9005,
        9006,
        "Windows.UI.Xaml.Controls.Border".into(),
        "AcrylicOverlay".into(),
        701,
    );
    // Companion side panel
    idx.on_add_on_thread(
        9001,
        9010,
        "Windows.UI.Xaml.Controls.Border".into(),
        "RightCompanionDropShadow".into(),
        701,
    );
    idx.on_add_on_thread(
        9001,
        9011,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "CompanionRoot".into(),
        701,
    );
    idx.on_add_on_thread(
        9011,
        9012,
        "Windows.UI.Xaml.Controls.Border".into(),
        "AcrylicBorder".into(),
        701,
    );
    // Inner button/item borders must NEVER match
    idx.on_add_on_thread(
        9005,
        9020,
        "Windows.UI.Xaml.Controls.Button".into(),
        "SearchBoxToggleButton".into(),
        701,
    );
    idx.on_add_on_thread(
        9020,
        9021,
        "Windows.UI.Xaml.Controls.Border".into(),
        "AcrylicBorder".into(),
        701,
    );
    // Power / Pinned item context menu inside StartMenu
    idx.on_add_on_thread(
        0,
        9030,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        701,
    );
    idx.on_add_on_thread(
        9030,
        9031,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        701,
    );

    assert_eq!(
        idx.classify_target_in_scope(9002, TapHostScope::StartMenu),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9004, TapHostScope::StartMenu),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9006, TapHostScope::StartMenu),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9010, TapHostScope::StartMenu),
        Some(TargetKind::ShellCardSubSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9012, TapHostScope::StartMenu),
        Some(TargetKind::ShellCardSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9021, TapHostScope::StartMenu),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(9031, TapHostScope::StartMenu),
        Some(TargetKind::MenuFlyoutSurface)
    );
}

#[test]
fn start_menu_targets_folder_modal_and_expanded_folder_list_subpanels() {
    let mut idx = VisualTreeIndex::new();

    // 1. Category / Unified FolderModal (StartFolderModal) > Grid#Root > LightDismissLayer + nameless Border + ContentControl
    idx.on_add_on_thread(0, 9100, "StartMenu.FolderModal".into(), "StartFolderModal".into(), 702);
    idx.on_add_on_thread(9100, 9101, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 702);
    idx.on_add_on_thread(
        9101,
        9102,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "LightDismissLayer".into(),
        702,
    );
    idx.on_add_on_thread(9101, 9103, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 702);
    idx.on_add_on_thread(
        9101,
        9104,
        "Windows.UI.Xaml.Controls.ContentControl".into(),
        "ContentControl".into(),
        702,
    );
    idx.on_add_on_thread(
        9104,
        9105,
        "Windows.UI.Xaml.Controls.GridView".into(),
        "LevelOneGridView".into(),
        702,
    );
    idx.on_add_on_thread(
        9105,
        9106,
        "Windows.UI.Xaml.Controls.GridViewItem".into(),
        "".into(),
        702,
    );
    idx.on_add_on_thread(9106, 9107, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 702);
    idx.on_add_on_thread(9107, 9108, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 702);

    // 2. Named FolderModal variant inside inner frame
    idx.on_add_on_thread(0, 9110, "StartMenu.FolderModal".into(), "FolderModal".into(), 702);
    idx.on_add_on_thread(9110, 9111, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 702);
    idx.on_add_on_thread(9111, 9112, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 702);

    // 3. Pinned ExpandedFolderList subpanel + inner control Border isolation
    idx.on_add_on_thread(
        0,
        9120,
        "StartMenu.ExpandedFolderList".into(),
        "".into(),
        702,
    );
    idx.on_add_on_thread(9120, 9121, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 702);
    idx.on_add_on_thread(9121, 9122, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 702);
    idx.on_add_on_thread(9121, 9123, "Windows.UI.Xaml.Controls.Grid".into(), "".into(), 702);
    idx.on_add_on_thread(
        9123,
        9124,
        "Windows.UI.Xaml.Controls.TextBox".into(),
        "ExpandedFolderNameTextBox".into(),
        702,
    );
    idx.on_add_on_thread(9124, 9125, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 702);
    idx.on_add_on_thread(9125, 9126, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 702);

    // 4. Out-of-order tree insertion: Border arrives before Grid#Root and FolderModal
    idx.on_add_on_thread(9131, 9132, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 703);
    assert_eq!(
        idx.classify_target_in_scope(9132, TapHostScope::StartMenu),
        None
    );
    idx.on_add_on_thread(9130, 9131, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 703);
    assert_eq!(
        idx.classify_target_in_scope(9132, TapHostScope::StartMenu),
        None
    );
    idx.on_add_on_thread(0, 9130, "StartMenu.FolderModal".into(), "StartFolderModal".into(), 703);
    assert_eq!(
        idx.classify_target_in_scope(9132, TapHostScope::StartMenu),
        Some(TargetKind::StartMenuModalSurface)
    );

    // Self-created sibling overlays must NEVER match
    idx.on_add_on_thread(
        9101,
        9140,
        "Windows.UI.Xaml.Controls.Grid".into(),
        START_MODAL_OVERLAY_NAME.into(),
        702,
    );
    idx.on_add_on_thread(9140, 9141, "Windows.UI.Xaml.Controls.Border".into(), "".into(), 702);
    idx.on_add_on_thread(
        9101,
        9142,
        "Windows.UI.Xaml.Controls.Border".into(),
        START_MENU_OVERLAY_NAME.into(),
        702,
    );

    assert_eq!(
        idx.classify_target_in_scope(9102, TapHostScope::StartMenu),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(9140, TapHostScope::StartMenu),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(9141, TapHostScope::StartMenu),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(9142, TapHostScope::StartMenu),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(9103, TapHostScope::StartMenu),
        Some(TargetKind::StartMenuModalSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9108, TapHostScope::StartMenu),
        None
    );
    assert_eq!(
        idx.classify_target_in_scope(9112, TapHostScope::StartMenu),
        Some(TargetKind::StartMenuModalSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9122, TapHostScope::StartMenu),
        Some(TargetKind::StartMenuModalSurface)
    );
    assert_eq!(
        idx.classify_target_in_scope(9126, TapHostScope::StartMenu),
        None
    );
}

#[test]
fn visual_tree_index_removes_deep_subtrees_reparents_and_guards_cycles() {
    // 1. Deep 5000-node tree removes every descendant without 4096-node orphan leakage
    let mut idx = VisualTreeIndex::new();
    idx.on_add(0, 1, "Root".into(), "".into());
    for id in 2..=5000u64 {
        idx.on_add(id - 1, id, "Node".into(), "".into());
    }
    assert_eq!(idx.node_count(), 5000);
    let removed = idx.on_remove(1);
    assert_eq!(removed.len(), 5000);
    assert_eq!(idx.node_count(), 0);

    // 2. Re-parenting detaches child from old parent and binds to new parent
    let mut idx = VisualTreeIndex::new();
    idx.on_add(0, 10, "OldParent".into(), "".into());
    idx.on_add(0, 20, "NewParent".into(), "".into());
    idx.on_add(10, 30, "Child".into(), "".into());
    idx.on_add(20, 30, "Child".into(), "".into());
    let rem_old = idx.on_remove(10);
    assert_eq!(rem_old, vec![10]);
    assert_eq!(idx.node_count(), 2);
    let mut rem_new = idx.on_remove(20);
    rem_new.sort_unstable();
    assert_eq!(rem_new, vec![20, 30]);
    assert_eq!(idx.node_count(), 0);

    // 3. Cycle A -> B -> A terminates cleanly without infinite loop
    let mut idx = VisualTreeIndex::new();
    idx.on_add(2, 1, "NodeA".into(), "".into());
    idx.on_add(1, 2, "NodeB".into(), "".into());
    let mut rem_cycle = idx.on_remove(1);
    rem_cycle.sort_unstable();
    assert_eq!(rem_cycle, vec![1, 2]);
    assert_eq!(idx.node_count(), 0);
}

#[test]
fn search_host_targets_app_border_layers_shadow_and_excludes_search_box_and_hc() {
    let mut idx = VisualTreeIndex::new();
    idx.on_add_on_thread(
        0,
        1,
        "Cortana.UI.Views.TaskbarSearchPage".into(),
        "".into(),
        9001,
    );
    idx.on_add_on_thread(1, 2, "Windows.UI.Xaml.Controls.Grid".into(), "RootGrid".into(), 9001);
    idx.on_add_on_thread(2, 3, "Windows.UI.Xaml.Controls.Grid".into(), "OuterBorderGrid".into(), 9001);
    idx.on_add_on_thread(3, 4, "Windows.UI.Xaml.Controls.Grid".into(), "BorderGrid".into(), 9001);
    idx.on_add_on_thread(4, 5, "Windows.UI.Xaml.Controls.Border".into(), "AppBorder".into(), 9001);
    idx.on_add_on_thread(4, 6, "Windows.UI.Xaml.Controls.Border".into(), "AccentAppBorder".into(), 9001);
    idx.on_add_on_thread(4, 7, "Windows.UI.Xaml.Controls.Border".into(), "LayerBorder".into(), 9001);
    idx.on_add_on_thread(4, 8, "Windows.UI.Xaml.Controls.Border".into(), "AccentLayerBorder".into(), 9001);
    idx.on_add_on_thread(4, 9, "Windows.UI.Xaml.Controls.Border".into(), "dropshadow".into(), 9001);

    // Search box & QueryFormulation WebView must stay untouched
    idx.on_add_on_thread(2, 10, "Windows.UI.Xaml.Controls.Border".into(), "TaskbarSearchBackground".into(), 9001);
    idx.on_add_on_thread(2, 11, "Cortana.UI.Views.RichSearchBoxControl".into(), "SearchBoxControl".into(), 9001);
    idx.on_add_on_thread(11, 12, "Windows.UI.Xaml.Controls.Grid".into(), "RootGrid".into(), 9001);
    idx.on_add_on_thread(12, 13, "Windows.UI.Xaml.Controls.Border".into(), "BorderElement".into(), 9001);
    idx.on_add_on_thread(2, 14, "Windows.UI.Xaml.Controls.Grid".into(), "QueryFormulationRoot".into(), 9001);
    idx.on_add_on_thread(14, 15, "Cortana.UI.Views.HostedWebView2Control".into(), "QueryFormulationHostedWebView2".into(), 9001);
    idx.on_add_on_thread(15, 16, "Windows.UI.Xaml.Controls.Grid".into(), "WebViewGrid".into(), 9001);

    // High-contrast borders must stay untouched
    idx.on_add_on_thread(2, 17, "Windows.UI.Xaml.Controls.Grid".into(), "HCOuterBorderGrid".into(), 9001);
    idx.on_add_on_thread(17, 18, "Windows.UI.Xaml.Controls.Border".into(), "HCBorder".into(), 9001);

    // Context MenuFlyoutPresenter inside SearchHost
    idx.on_add_on_thread(0, 19, "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(), "".into(), 9001);
    idx.on_add_on_thread(19, 20, "Windows.UI.Xaml.Controls.Border".into(), "OuterBorder".into(), 9001);

    assert_eq!(idx.classify_target_in_scope(2, TapHostScope::SearchHost), None);
    assert_eq!(idx.classify_target_in_scope(4, TapHostScope::SearchHost), None);
    assert_eq!(idx.classify_target_in_scope(5, TapHostScope::SearchHost), Some(TargetKind::ShellCardSurface));
    assert_eq!(idx.classify_target_in_scope(6, TapHostScope::SearchHost), Some(TargetKind::ShellCardSurface));
    assert_eq!(idx.classify_target_in_scope(7, TapHostScope::SearchHost), Some(TargetKind::ShellCardSubSurface));
    assert_eq!(idx.classify_target_in_scope(8, TapHostScope::SearchHost), Some(TargetKind::ShellCardSubSurface));
    assert_eq!(idx.classify_target_in_scope(9, TapHostScope::SearchHost), Some(TargetKind::ShellCardSubSurface));
    assert_eq!(idx.classify_target_in_scope(10, TapHostScope::SearchHost), None);
    assert_eq!(idx.classify_target_in_scope(13, TapHostScope::SearchHost), None);
    assert_eq!(idx.classify_target_in_scope(16, TapHostScope::SearchHost), None);
    assert_eq!(idx.classify_target_in_scope(18, TapHostScope::SearchHost), None);
    assert_eq!(idx.classify_target_in_scope(20, TapHostScope::SearchHost), Some(TargetKind::MenuFlyoutSurface));
}
