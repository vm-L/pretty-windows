use super::*;
use std::collections::HashSet;

#[test]
fn isolates_taskbar_fill_and_stroke_rectangles_and_excludes_flyouts_and_start_menu() {
    let mut idx = VisualTreeIndex::new();
    // Primary monitor taskbar tree
    idx.on_add(0, 1, "Taskbar.TaskbarFrame".into(), "TaskbarFrame".into());
    idx.on_add(
        1,
        2,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "RootGrid".into(),
    );
    idx.on_add(
        2,
        3,
        "Taskbar.TaskbarBackground".into(),
        "BackgroundControl".into(),
    );
    idx.on_add(3, 4, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
    idx.on_add(
        4,
        5,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );
    idx.on_add(
        4,
        6,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundStroke".into(),
    );
    // Taskbar app icons sibling must never be matched
    idx.on_add(2, 7, "Taskbar.TaskListLabeledButtonPanel".into(), "".into());

    // Hover flyout nested under TaskbarFrame must NOT match
    idx.on_add(
        2,
        8,
        "Taskbar.TaskbarBackground".into(),
        "HoverFlyoutBackgroundControl".into(),
    );
    idx.on_add(8, 9, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
    idx.on_add(
        9,
        10,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );

    // Win11 24H2 wrapped Secondary monitor taskbar tree (Border -> ScrollViewer -> ScrollContentPresenter -> TaskbarFrame)
    idx.on_add(
        0,
        20,
        "Windows.UI.Xaml.Controls.Border".into(),
        "OuterBorder".into(),
    );
    idx.on_add(
        20,
        21,
        "Windows.UI.Xaml.Controls.ScrollViewer".into(),
        "".into(),
    );
    idx.on_add(
        21,
        22,
        "Windows.UI.Xaml.Controls.ScrollContentPresenter".into(),
        "".into(),
    );
    idx.on_add(
        22,
        23,
        "Taskbar.TaskbarFrame".into(),
        "TaskbarFrame".into(),
    );
    idx.on_add(
        23,
        24,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "RootGrid".into(),
    );
    idx.on_add(
        24,
        25,
        "Taskbar.TaskbarBackground".into(),
        "BackgroundControl".into(),
    );
    idx.on_add(25, 26, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
    idx.on_add(
        26,
        27,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );
    idx.on_add(
        26,
        28,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundStroke".into(),
    );

    // Start menu element with identical child names must NOT match
    idx.on_add(0, 100, "StartMenu.StartInnerFrame".into(), "".into());
    idx.on_add(
        100,
        101,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );
    idx.on_add(
        100,
        102,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundStroke".into(),
    );

    // ScreenEdgeStroke directly under TaskbarBackground (even before TaskbarFrame attaches above)
    idx.on_add(0, 30, "Taskbar.TaskbarBackground".into(), "TaskbarBG".into());
    idx.on_add(
        30,
        31,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "ScreenEdgeStroke".into(),
    );

    // Out-of-order add: child BackgroundFill arrives before parent TaskbarBackground arrives
    idx.on_add(
        41,
        42,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );
    assert!(!idx.is_taskbar_bg_target(42));
    idx.on_add(0, 41, "Taskbar.TaskbarBackground".into(), "LateBG".into());
    assert!(idx.is_taskbar_bg_target(42));

    let targets: HashSet<u64> = idx.collect_targets().into_iter().collect();
    assert_eq!(targets, HashSet::from([5, 6, 27, 28, 31, 42]));
    assert!(idx.node_count() >= 15);
}

#[test]
fn subtree_remove_reparent_and_cycle_guard_behave_safely() {
    let mut idx = VisualTreeIndex::new();
    idx.on_add(0, 1, "Taskbar.TaskbarFrame".into(), "".into());
    idx.on_add(1, 2, "Taskbar.TaskbarBackground".into(), "".into());
    idx.on_add(
        2,
        3,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );
    // Secondary TaskbarFrame
    idx.on_add(0, 20, "Taskbar.TaskbarFrame".into(), "".into());
    idx.on_add(
        2,
        4,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundStroke".into(),
    );
    // Reparent node 4 before removing subtree 1
    idx.on_add(
        20,
        4,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundStroke".into(),
    );

    let removed = idx.on_remove(1);
    assert!(removed.contains(&1) && removed.contains(&2) && removed.contains(&3));
    assert!(!removed.contains(&4));
    assert_eq!(idx.collect_targets(), vec![4]);

    // Cycle protection (even when cycle passes through TaskbarFrame)
    idx.on_add(52, 50, "Taskbar.TaskbarFrame".into(), "".into());
    idx.on_add(50, 52, "Windows.UI.Xaml.Controls.Grid".into(), "".into());
    idx.on_add(
        52,
        51,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
    );
    assert!(!idx.is_taskbar_bg_target(51));
}

#[test]
fn classifies_overflow_border_and_panel_while_excluding_icon_tiles() {
    let mut idx = VisualTreeIndex::new();
    // 1. Standard Win11 overflow flyout tree: NotificationAreaOverflow > Grid#OverflowRootGrid > Border
    idx.on_add(
        0,
        200,
        "SystemTray.NotificationAreaOverflow".into(),
        "OverflowView".into(),
    );
    idx.on_add(
        200,
        201,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "OverflowRootGrid".into(),
    );
    idx.on_add(201, 202, "Windows.UI.Xaml.Controls.Border".into(), "".into());
    idx.on_add(
        200,
        203,
        "Windows.UI.Xaml.Controls.Border".into(),
        "OverflowFlyoutBackgroundBorder".into(),
    );

    // 2. Child icon buttons & tiles inside OverflowRootGrid > Border must NEVER be touched
    idx.on_add(
        202,
        210,
        "Windows.UI.Xaml.Controls.ItemsControl".into(),
        "OverflowItemsControl".into(),
    );
    idx.on_add(
        210,
        211,
        "Windows.UI.Xaml.Controls.ItemsPresenter".into(),
        "".into(),
    );
    idx.on_add(
        211,
        212,
        "Windows.UI.Xaml.Controls.WrapGrid".into(),
        "".into(),
    );
    idx.on_add(
        212,
        213,
        "SystemTray.NotifyIconView".into(),
        "".into(),
    );
    idx.on_add(213, 214, "Windows.UI.Xaml.Controls.Button".into(), "".into());
    idx.on_add(
        214,
        215,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
    );

    // 3. Out-of-order add: background border arrives before OverflowRootGrid parent arrives
    idx.on_add(230, 231, "Windows.UI.Xaml.Controls.Border".into(), "".into());
    assert_eq!(idx.classify_target(231), None);
    idx.on_add(
        0,
        230,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "OverflowRootGrid".into(),
    );

    assert_eq!(idx.classify_target(201), Some(TargetKind::OverflowPanel));
    assert_eq!(idx.classify_target(202), Some(TargetKind::OverflowBorder));
    assert_eq!(idx.classify_target(203), Some(TargetKind::OverflowBorder));
    assert_eq!(idx.classify_target(230), Some(TargetKind::OverflowPanel));
    assert_eq!(idx.classify_target(231), Some(TargetKind::OverflowBorder));
    assert_eq!(idx.classify_target(215), None);

    let classified: HashSet<(u64, TargetKind)> =
        idx.collect_classified_targets().into_iter().collect();
    assert_eq!(
        classified,
        HashSet::from([
            (201, TargetKind::OverflowPanel),
            (202, TargetKind::OverflowBorder),
            (203, TargetKind::OverflowBorder),
            (230, TargetKind::OverflowPanel),
            (231, TargetKind::OverflowBorder),
        ])
    );
}

#[test]
fn collect_classified_targets_for_thread_isolates_ui_threads() {
    let mut idx = VisualTreeIndex::new();
    idx.on_add_on_thread(0, 1, "Taskbar.TaskbarFrame".into(), "TaskbarFrame".into(), 101);
    idx.on_add_on_thread(
        1,
        2,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        101,
    );

    idx.on_add_on_thread(
        0,
        10,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "OverflowRootGrid".into(),
        202,
    );
    idx.on_add_on_thread(
        10,
        11,
        "Windows.UI.Xaml.Controls.Border".into(),
        "".into(),
        202,
    );

    let t1: HashSet<(u64, TargetKind)> =
        idx.collect_classified_targets_for_thread(101).into_iter().collect();
    let t2: HashSet<(u64, TargetKind)> =
        idx.collect_classified_targets_for_thread(202).into_iter().collect();

    assert_eq!(t1, HashSet::from([(2, TargetKind::ShapeElement)]));
    assert_eq!(
        t2,
        HashSet::from([
            (10, TargetKind::OverflowPanel),
            (11, TargetKind::OverflowBorder),
        ])
    );
}

#[test]
fn classifies_alttab_background_element_and_excludes_items_and_taskview() {
    let mut idx = VisualTreeIndex::new();
    // 1. Alt+Tab visual tree: AltTab > Grid#ModalRootGrid > Border#BackgroundElement
    idx.on_add(
        0,
        400,
        "WindowsInternal.ComposableShell.Experiences.Switcher.AltTab".into(),
        "AltTabRoot".into(),
    );
    idx.on_add(
        400,
        401,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
    );
    idx.on_add(
        401,
        402,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundElement".into(),
    );
    // Item cards inside SwitchItemList / SwitchItemControl must NEVER be touched
    idx.on_add(
        402,
        410,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemList".into(),
        "SwitchItemListControl".into(),
    );
    idx.on_add(
        410,
        411,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemControl".into(),
        "".into(),
    );
    idx.on_add(
        411,
        412,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
    );

    // 2. Out-of-order add: BackgroundElement arrives before AltTabRoot attaches above
    idx.on_add(
        420,
        421,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
    );
    idx.on_add(
        421,
        422,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundElement".into(),
    );
    assert_eq!(idx.classify_target(422), Some(TargetKind::AltTabBorder));

    // 3. TaskView and SnapAssist with identically named borders must NOT match
    idx.on_add(
        0,
        450,
        "WindowsInternal.ComposableShell.Experiences.Switcher.TaskView".into(),
        "TaskViewRoot".into(),
    );
    idx.on_add(
        450,
        451,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
    );
    idx.on_add(
        451,
        452,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundElement".into(),
    );
    idx.on_add(
        0,
        460,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SnapAssist".into(),
        "SnapAssistRoot".into(),
    );
    idx.on_add(
        460,
        461,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
    );
    idx.on_add(
        461,
        462,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundElement".into(),
    );

    assert_eq!(idx.classify_target(402), Some(TargetKind::AltTabBorder));
    assert_eq!(idx.classify_target(412), None);
    assert_eq!(idx.classify_target(452), None);
    assert_eq!(idx.classify_target(462), None);
}

#[test]
fn classifies_alttab_item_background_border_while_excluding_focus_and_taskview() {
    let mut idx = VisualTreeIndex::new();
    // Alt+Tab item container visual tree:
    // AltTabRoot -> ModalRootGrid -> BackgroundElement -> SwitchItemList -> ScrollViewer
    // -> SwitchItemListViewItem -> Grid#Root -> Border#BackgroundBorder / Border#FocusRect
    idx.on_add_on_thread(
        0,
        500,
        "WindowsInternal.ComposableShell.Experiences.Switcher.AltTab".into(),
        "AltTabRoot".into(),
        77,
    );
    idx.on_add_on_thread(
        500,
        501,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
        77,
    );
    idx.on_add_on_thread(
        501,
        502,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundElement".into(),
        77,
    );
    idx.on_add_on_thread(
        502,
        503,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemList".into(),
        "SwitchItemListControl".into(),
        77,
    );
    idx.on_add_on_thread(
        503,
        504,
        "Windows.UI.Xaml.Controls.ScrollViewer".into(),
        "".into(),
        77,
    );
    idx.on_add_on_thread(
        504,
        505,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemListViewItem".into(),
        "".into(),
        77,
    );
    idx.on_add_on_thread(505, 506, "Windows.UI.Xaml.Controls.Grid".into(), "Root".into(), 77);
    idx.on_add_on_thread(
        506,
        507,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        77,
    );
    idx.on_add_on_thread(
        506,
        508,
        "Windows.UI.Xaml.Controls.Border".into(),
        "FocusRect".into(),
        77,
    );

    // TaskView / SnapAssist / TaskbarSwitcher SwitchItemElement must NOT match AltTabItemBorder.
    idx.on_add_on_thread(
        0,
        600,
        "WindowsInternal.ComposableShell.Experiences.Switcher.TaskView".into(),
        "TaskViewRoot".into(),
        88,
    );
    idx.on_add_on_thread(
        600,
        601,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
        88,
    );
    idx.on_add_on_thread(
        601,
        602,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemElement".into(),
        "".into(),
        88,
    );
    idx.on_add_on_thread(
        602,
        603,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        88,
    );
    idx.on_add_on_thread(
        505,
        604,
        "WindowsInternal.ComposableShell.Experiences.Switcher.TaskbarSwitcher".into(),
        "TaskbarSwitcherRoot".into(),
        77,
    );
    idx.on_add_on_thread(
        604,
        605,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        77,
    );
    idx.on_add_on_thread(
        601,
        606,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemListViewItem".into(),
        "".into(),
        88,
    );

    // Out-of-order subtree addition: BackgroundBorder arrives before SwitchItemListViewItem attaches to thread container.
    idx.on_add_on_thread(
        700,
        701,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "Root".into(),
        77,
    );
    idx.on_add_on_thread(
        701,
        702,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        77,
    );
    assert_eq!(idx.classify_target(702), None);
    idx.on_add_on_thread(
        504,
        700,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemListViewItem".into(),
        "".into(),
        77,
    );

    assert!(idx.thread_has_alttab_container(77));
    assert!(!idx.thread_has_alttab_container(88));
    assert_eq!(idx.classify_target(502), Some(TargetKind::AltTabBorder));
    assert_eq!(idx.classify_target(505), Some(TargetKind::AltTabItemFocus));
    assert_eq!(idx.classify_target(507), Some(TargetKind::AltTabItemBorder));
    assert_eq!(idx.classify_target(508), None);
    assert_eq!(idx.classify_target(603), None);
    assert_eq!(idx.classify_target(605), None);
    assert_eq!(idx.classify_target(606), None);
    assert_eq!(idx.classify_target(700), Some(TargetKind::AltTabItemFocus));
    assert_eq!(idx.classify_target(702), Some(TargetKind::AltTabItemBorder));
}

#[test]
fn classifies_hover_flyout_background_and_shapes_while_excluding_thumbnail_items() {
    let mut idx = VisualTreeIndex::new();
    // 1. Standard Win11 FlyoutFrame visual tree
    idx.on_add_on_thread(0, 800, "Taskbar.FlyoutFrame".into(), "FlyoutFrame".into(), 4060);
    idx.on_add_on_thread(
        800,
        801,
        "Windows.UI.Xaml.Controls.Canvas".into(),
        "HoverFlyoutCanvas".into(),
        4060,
    );
    idx.on_add_on_thread(
        801,
        802,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "HoverFlyoutGrid".into(),
        4060,
    );
    idx.on_add_on_thread(
        802,
        803,
        "Windows.UI.Xaml.Controls.Border".into(),
        "HoverFlyoutBackground".into(),
        4060,
    );
    idx.on_add_on_thread(
        803,
        804,
        "Taskbar.TaskbarBackground".into(),
        "HoverFlyoutBackgroundControl".into(),
        4060,
    );
    idx.on_add_on_thread(804, 805, "Windows.UI.Xaml.Controls.Grid".into(), "".into(), 4060);
    idx.on_add_on_thread(
        805,
        806,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        4060,
    );
    idx.on_add_on_thread(
        805,
        807,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundStroke".into(),
        4060,
    );
    idx.on_add_on_thread(
        805,
        808,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "ScreenEdgeStroke".into(),
        4060,
    );

    // 2. Content inside HoverFlyoutContent (thumbnail cards & overflow list) must NEVER be touched
    idx.on_add_on_thread(
        803,
        810,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "HoverFlyoutContent".into(),
        4060,
    );
    idx.on_add_on_thread(
        810,
        811,
        "Taskbar.TaskItemThumbnailList".into(),
        "TaskItemThumbnailList".into(),
        4060,
    );
    idx.on_add_on_thread(
        811,
        812,
        "Taskbar.TaskItemThumbnailView".into(),
        "".into(),
        4060,
    );
    idx.on_add_on_thread(
        812,
        813,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        4060,
    );
    idx.on_add_on_thread(
        812,
        814,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        4060,
    );
    idx.on_add_on_thread(
        810,
        815,
        "Taskbar.OverflowFlyoutList".into(),
        "".into(),
        4060,
    );
    idx.on_add_on_thread(
        815,
        816,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        4060,
    );

    // 3. Out-of-order add: BackgroundFill under HoverFlyoutBackgroundControl arrives before FlyoutFrame
    idx.on_add_on_thread(
        820,
        821,
        "Taskbar.TaskbarBackground".into(),
        "HoverFlyoutBackgroundControl".into(),
        4060,
    );
    idx.on_add_on_thread(
        821,
        822,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        4060,
    );
    assert_eq!(idx.classify_target(822), Some(TargetKind::HoverFlyoutShape));
    assert!(!idx.is_taskbar_bg_target(822));

    // 4. Generic MenuFlyout / ToolTip with identically named rectangles must NOT match
    idx.on_add_on_thread(
        0,
        830,
        "Windows.UI.Xaml.Controls.MenuFlyout".into(),
        "".into(),
        4060,
    );
    idx.on_add_on_thread(
        830,
        831,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        4060,
    );
    idx.on_add_on_thread(
        0,
        832,
        "Windows.UI.Xaml.Controls.ToolTip".into(),
        "".into(),
        4060,
    );
    idx.on_add_on_thread(
        832,
        833,
        "Windows.UI.Xaml.Shapes.Rectangle".into(),
        "BackgroundFill".into(),
        4060,
    );

    assert_eq!(idx.classify_target(803), Some(TargetKind::HoverFlyoutBorder));
    assert_eq!(idx.classify_target(806), Some(TargetKind::HoverFlyoutShape));
    assert_eq!(idx.classify_target(807), Some(TargetKind::HoverFlyoutShape));
    assert_eq!(idx.classify_target(808), Some(TargetKind::HoverFlyoutShape));
    assert!(!idx.is_taskbar_bg_target(806));
    assert_eq!(idx.classify_target(813), None);
    assert_eq!(idx.classify_target(814), None);
    assert_eq!(idx.classify_target(816), None);
    assert_eq!(idx.classify_target(831), None);
    assert_eq!(idx.classify_target(833), None);
}

#[test]
fn ignores_alttab_item_overlay_border_while_classifying_native_background_border() {
    let mut idx = VisualTreeIndex::new();
    idx.on_add_on_thread(
        0,
        900,
        "WindowsInternal.ComposableShell.Experiences.Switcher.AltTab".into(),
        "AltTabRoot".into(),
        1234,
    );
    idx.on_add_on_thread(
        900,
        901,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ModalRootGrid".into(),
        1234,
    );
    idx.on_add_on_thread(
        901,
        902,
        "WindowsInternal.ComposableShell.Experiences.Switcher.SwitchItemList".into(),
        "SwitchItemList".into(),
        1234,
    );
    idx.on_add_on_thread(
        902,
        903,
        "Windows.UI.Xaml.Controls.ListViewItem".into(),
        "".into(),
        1234,
    );
    idx.on_add_on_thread(
        903,
        904,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "Root".into(),
        1234,
    );
    idx.on_add_on_thread(
        904,
        905,
        "Windows.UI.Xaml.Controls.Border".into(),
        ALTTAB_ITEM_OVERLAY_NAME.into(),
        1234,
    );
    idx.on_add_on_thread(
        904,
        906,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        1234,
    );

    assert_eq!(idx.classify_target(905), None);
    assert_eq!(idx.classify_target(906), Some(TargetKind::AltTabItemBorder));
}

#[test]
fn classifies_osd_main_grid_and_excludes_inner_controls() {
    let mut idx = VisualTreeIndex::new();
    // 1. Standard Win11 HWConfirmatorUI tree:
    // ConfirmatorHostControl > Canvas#ConfirmatorFrameCanvas > Grid > Grid#ConfirmatorMainGrid
    idx.on_add_on_thread(
        0,
        1000,
        "HWConfirmatorUI.ConfirmatorHostControl".into(),
        "".into(),
        5724,
    );
    idx.on_add_on_thread(
        1000,
        1001,
        "Windows.UI.Xaml.Controls.Canvas".into(),
        "ConfirmatorFrameCanvas".into(),
        5724,
    );
    idx.on_add_on_thread(
        1001,
        1002,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "".into(),
        5724,
    );
    idx.on_add_on_thread(
        1002,
        1003,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ConfirmatorMainGrid".into(),
        5724,
    );
    // Inner OSD controls must never be matched
    idx.on_add_on_thread(
        1003,
        1004,
        "HWConfirmatorUI.CustomThumbSlider".into(),
        "volumeSlider".into(),
        5724,
    );
    idx.on_add_on_thread(
        1003,
        1005,
        "Windows.UI.Xaml.Controls.Button".into(),
        "MuteButton".into(),
        5724,
    );
    idx.on_add_on_thread(
        1003,
        1006,
        "Windows.UI.Xaml.Controls.TextBlock".into(),
        "volumeLevelText".into(),
        5724,
    );
    idx.on_add_on_thread(
        1005,
        1007,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        5724,
    );

    // 2. Out-of-order add: ConfirmatorMainGrid arrives before ConfirmatorFrameCanvas attaches
    idx.on_add_on_thread(
        1020,
        1021,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ConfirmatorMainGrid".into(),
        5724,
    );
    assert_eq!(idx.classify_target(1021), Some(TargetKind::OsdPanel));
    idx.on_add_on_thread(
        0,
        1020,
        "Windows.UI.Xaml.Controls.Canvas".into(),
        "ConfirmatorFrameCanvas".into(),
        5724,
    );

    // 3. Nested under forbidden node (e.g. StartMenu) or unrelated Grid must NOT match
    idx.on_add_on_thread(
        0,
        1030,
        "StartMenu.StartInnerFrame".into(),
        "StartMenu".into(),
        5724,
    );
    idx.on_add_on_thread(
        1030,
        1031,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "ConfirmatorMainGrid".into(),
        5724,
    );

    assert_eq!(idx.classify_target(1002), None);
    assert_eq!(idx.classify_target(1003), Some(TargetKind::OsdPanel));
    assert_eq!(idx.classify_target(1004), None);
    assert_eq!(idx.classify_target(1005), None);
    assert_eq!(idx.classify_target(1006), None);
    assert_eq!(idx.classify_target(1007), None);
    assert_eq!(idx.classify_target(1021), Some(TargetKind::OsdPanel));
    assert_eq!(idx.classify_target(1031), None);
}

#[test]
fn classifies_tooltip_layout_root_and_rejects_menuflyout_or_non_tooltip_presenters() {
    let mut idx = VisualTreeIndex::new();

    // 1. Standard Win11 XAML ToolTip > ContentPresenter#LayoutRoot > TextBlock
    idx.on_add_on_thread(
        0,
        2000,
        "Windows.UI.Xaml.Controls.Primitives.PopupRoot".into(),
        "PopupRoot".into(),
        25024,
    );
    idx.on_add_on_thread(
        2000,
        2001,
        "Windows.UI.Xaml.Controls.ToolTip".into(),
        "".into(),
        25024,
    );
    idx.on_add_on_thread(
        2001,
        2002,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "LayoutRoot".into(),
        25024,
    );
    idx.on_add_on_thread(
        2002,
        2003,
        "Windows.UI.Xaml.Controls.TextBlock".into(),
        "".into(),
        25024,
    );

    // 2. Out-of-order add: LayoutRoot arrives before parent ToolTip
    idx.on_add_on_thread(
        2020,
        2021,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "LayoutRoot".into(),
        25024,
    );
    assert_eq!(idx.classify_target(2021), None);
    idx.on_add_on_thread(
        0,
        2020,
        "Windows.UI.Xaml.Controls.ToolTip".into(),
        "TrayToolTip".into(),
        25024,
    );
    assert_eq!(idx.classify_target(2021), Some(TargetKind::ToolTipPresenter));

    // 3. ContentPresenter#LayoutRoot under MenuFlyoutPresenter, Button, or StartMenu must NEVER match
    idx.on_add_on_thread(
        0,
        2030,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        25024,
    );
    idx.on_add_on_thread(
        2030,
        2031,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "LayoutRoot".into(),
        25024,
    );
    idx.on_add_on_thread(
        0,
        2040,
        "Windows.UI.Xaml.Controls.Button".into(),
        "".into(),
        25024,
    );
    idx.on_add_on_thread(
        2040,
        2041,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "LayoutRoot".into(),
        25024,
    );
    idx.on_add_on_thread(
        0,
        2050,
        "StartMenu.StartInnerFrame".into(),
        "StartMenu".into(),
        25024,
    );
    idx.on_add_on_thread(
        2050,
        2051,
        "Windows.UI.Xaml.Controls.ToolTip".into(),
        "".into(),
        25024,
    );
    idx.on_add_on_thread(
        2051,
        2052,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "LayoutRoot".into(),
        25024,
    );

    assert_eq!(idx.classify_target(2001), None);
    assert_eq!(idx.classify_target(2002), Some(TargetKind::ToolTipPresenter));
    assert_eq!(idx.classify_target(2003), None);
    assert_eq!(idx.classify_target(2031), None);
    assert_eq!(idx.classify_target(2041), None);
    assert_eq!(idx.classify_target(2052), None);
}

#[test]
fn classifies_menu_flyout_and_jumplist_surfaces_with_strict_isolation() {
    let mut idx = VisualTreeIndex::new();

    // 1. Standard cascading MenuFlyouts: main menu (3001) + submenu (3011)
    idx.on_add_on_thread(
        0,
        3000,
        "Windows.UI.Xaml.Controls.Primitives.PopupRoot".into(),
        "".into(),
        100,
    );
    idx.on_add_on_thread(
        3000,
        3001,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        100,
    );
    idx.on_add_on_thread(
        3001,
        3002,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        100,
    );
    idx.on_add_on_thread(
        3002,
        3003,
        "Windows.UI.Xaml.Controls.MenuFlyoutItem".into(),
        "".into(),
        100,
    );
    idx.on_add_on_thread(
        3003,
        3004,
        "Windows.UI.Xaml.Controls.Border".into(),
        "LayoutRoot".into(),
        100,
    );

    // Out-of-order submenu border arrives before presenter
    idx.on_add_on_thread(
        3011,
        3012,
        "Windows.UI.Xaml.Controls.Border".into(),
        "LayoutRoot".into(),
        100,
    );
    idx.on_add_on_thread(
        3000,
        3011,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        100,
    );

    // 2. Standalone JumpList surface
    idx.on_add_on_thread(
        3000,
        3020,
        "JumpViewUI.JumpListControl".into(),
        "JumpList".into(),
        100,
    );
    idx.on_add_on_thread(
        3020,
        3021,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "RootGrid".into(),
        100,
    );
    idx.on_add_on_thread(
        3021,
        3022,
        "JumpViewUI.JumpListItemControl".into(),
        "".into(),
        100,
    );
    idx.on_add_on_thread(
        3022,
        3023,
        "Windows.UI.Xaml.Controls.Border".into(),
        "BackgroundBorder".into(),
        100,
    );

    // 3. Forbidden containers (StartMenu, ComboBox, ToolTip) must never match
    idx.on_add_on_thread(0, 3030, "StartMenu.StartInnerView".into(), "StartMenu".into(), 100);
    idx.on_add_on_thread(
        3030,
        3031,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        100,
    );
    idx.on_add_on_thread(
        3031,
        3032,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        100,
    );
    idx.on_add_on_thread(0, 3040, "Windows.UI.Xaml.Controls.ComboBox".into(), "".into(), 100);
    idx.on_add_on_thread(
        3040,
        3041,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        100,
    );
    idx.on_add_on_thread(
        3041,
        3042,
        "Windows.UI.Xaml.Controls.Border".into(),
        "MenuFlyoutPresenterBorder".into(),
        100,
    );

    assert_eq!(idx.classify_target(3002), Some(TargetKind::MenuFlyoutSurface));
    assert_eq!(idx.classify_target(3012), Some(TargetKind::MenuFlyoutSurface));
    assert_eq!(idx.classify_target(3021), Some(TargetKind::MenuFlyoutSurface));
    assert_eq!(idx.classify_target(3004), None);
    assert_eq!(idx.classify_target(3023), None);
    assert_eq!(idx.classify_target(3032), None);
    assert_eq!(idx.classify_target(3042), None);
}

#[test]
fn classifies_virtual_desktop_bar_surfaces_and_ignores_inner_desktop_items() {
    let mut idx = VisualTreeIndex::new();

    // Standard Win11 TaskViewTimeline > VirtualDesktopBarElement > GridElement
    // with mutually exclusive lazy sibling Borders:
    // VirtualDesktopSwitcherBackground (Acrylic pill) and VirtualDesktopBarBackground (bottom bar).
    idx.on_add_on_thread(
        0,
        7000,
        "WindowsInternal.ComposableShell.Experiences.Switcher.TaskViewTimeline".into(),
        "TaskViewTimelineRoot".into(),
        501,
    );
    idx.on_add_on_thread(
        7000,
        7001,
        "WindowsInternal.ComposableShell.Experiences.Switcher.VirtualDesktopBarElement".into(),
        "VirtualDesktopBar".into(),
        501,
    );
    idx.on_add_on_thread(
        7001,
        7002,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "GridElement".into(),
        501,
    );
    idx.on_add_on_thread(
        7002,
        7003,
        "Windows.UI.Xaml.Controls.Border".into(),
        "VirtualDesktopSwitcherBackground".into(),
        501,
    );
    idx.on_add_on_thread(
        7002,
        7004,
        "Windows.UI.Xaml.Controls.Border".into(),
        "VirtualDesktopBarBackground".into(),
        501,
    );
    // Inner desktop thumbnail tile & New Desktop button must NEVER match
    idx.on_add_on_thread(
        7002,
        7005,
        "WindowsInternal.ComposableShell.Experiences.Switcher.VirtualDesktopListViewItem".into(),
        "".into(),
        501,
    );
    idx.on_add_on_thread(
        7005,
        7006,
        "Windows.UI.Xaml.Controls.Border".into(),
        "VirtualDesktopBarBackground".into(),
        501,
    );

    assert_eq!(
        idx.classify_target(7003),
        Some(TargetKind::VirtualDesktopBarSurface)
    );
    assert_eq!(
        idx.classify_target(7004),
        Some(TargetKind::VirtualDesktopBarSurface)
    );
    assert_eq!(idx.classify_target(7006), None);

    // Older Win11 build with only VirtualDesktopBarBackground (no outer VirtualDesktopSwitcherBackground)
    // automatically promotes VirtualDesktopBarBackground to primary VirtualDesktopBarSurface.
    idx.on_add_on_thread(
        0,
        7100,
        "WindowsInternal.ComposableShell.Experiences.Switcher.VirtualDesktopBarElement".into(),
        "VirtualDesktopBar".into(),
        502,
    );
    idx.on_add_on_thread(
        7100,
        7101,
        "Windows.UI.Xaml.Controls.Border".into(),
        "VirtualDesktopBarBackground".into(),
        502,
    );
    assert_eq!(
        idx.classify_target(7101),
        Some(TargetKind::VirtualDesktopBarSurface)
    );
}

#[test]
fn classifies_input_switcher_card_and_overlay_panel_while_excluding_items_and_touch_keyboard() {
    let mut idx = VisualTreeIndex::new();
    // 1. Full HeaderFooterInputSwitcherStyle tree:
    // InputSwitcher > ContentControl#FlatContent > ContentPresenter > unnamed Grid (card root)
    //   ├─ Grid#OverlayPanel > StackPanel#HeaderPanel + ListView#List > ListViewItem > InputSwitchListItemControl
    //   └─ HyperlinkButton#Footer
    idx.on_add_on_thread(
        0,
        1000,
        "WindowsInternal.ComposableShell.Experiences.TextInput.Common.InputSwitcher".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1000,
        1001,
        "Windows.UI.Xaml.Controls.ContentControl".into(),
        "FlatContent".into(),
        6611,
    );
    idx.on_add_on_thread(
        1001,
        1002,
        "Windows.UI.Xaml.Controls.ContentPresenter".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1002,
        1003,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "".into(),
        6611,
    );
    // Card root before OverlayPanel arrives -> not yet classified
    assert_eq!(idx.classify_target(1003), None);

    idx.on_add_on_thread(
        1003,
        1004,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "OverlayPanel".into(),
        6611,
    );
    idx.on_add_on_thread(
        1003,
        1005,
        "Windows.UI.Xaml.Controls.HyperlinkButton".into(),
        "Footer".into(),
        6611,
    );
    idx.on_add_on_thread(
        1004,
        1006,
        "Windows.UI.Xaml.Controls.StackPanel".into(),
        "HeaderPanel".into(),
        6611,
    );
    idx.on_add_on_thread(
        1004,
        1007,
        "Windows.UI.Xaml.Controls.ListView".into(),
        "List".into(),
        6611,
    );
    idx.on_add_on_thread(
        1007,
        1008,
        "Windows.UI.Xaml.Controls.ListViewItem".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1008,
        1009,
        "WindowsInternal.ComposableShell.Experiences.TextInput.Common.InputSwitchListItemControl".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1009,
        1010,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "Root".into(),
        6611,
    );
    idx.on_add_on_thread(
        1010,
        1011,
        "Windows.UI.Xaml.Controls.Border".into(),
        "".into(),
        6611,
    );
    // Nested structural grid inside OverlayPanel must never match
    idx.on_add_on_thread(
        1004,
        1012,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "".into(),
        6611,
    );
    // HeaderPanel / Footer children
    idx.on_add_on_thread(
        1006,
        1013,
        "Windows.UI.Xaml.Controls.Border".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1005,
        1014,
        "Windows.UI.Xaml.Controls.Border".into(),
        "".into(),
        6611,
    );

    assert_eq!(idx.classify_target(1003), Some(TargetKind::InputSwitcherPanel));
    assert_eq!(idx.classify_target(1004), Some(TargetKind::InputSwitcherSubPanel));
    assert_eq!(idx.classify_target(1005), None);
    assert_eq!(idx.classify_target(1006), None);
    assert_eq!(idx.classify_target(1010), None);
    assert_eq!(idx.classify_target(1011), None);
    assert_eq!(idx.classify_target(1012), None);
    assert_eq!(idx.classify_target(1013), None);
    assert_eq!(idx.classify_target(1014), None);

    // 2. Fallback matching via HeaderPanel + Footer direct children without OverlayPanel
    idx.on_add_on_thread(
        0,
        1020,
        "WindowsInternal.ComposableShell.Experiences.TextInput.Common.DirectionalAwareInputSwitcher".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1020,
        1021,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1021,
        1022,
        "Windows.UI.Xaml.Controls.StackPanel".into(),
        "HeaderPanel".into(),
        6611,
    );
    idx.on_add_on_thread(
        1021,
        1023,
        "Windows.UI.Xaml.Controls.HyperlinkButton".into(),
        "Footer".into(),
        6611,
    );
    assert_eq!(idx.classify_target(1021), Some(TargetKind::InputSwitcherPanel));

    // 3. DefaultInputSwitcherStyle / Touch Keyboard in-app switcher (OverlayGrid + Popup + List) must NOT match
    idx.on_add_on_thread(
        0,
        1030,
        "WindowsInternal.ComposableShell.Experiences.TextInput.Common.InputSwitcher".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1030,
        1031,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "OverlayGrid".into(),
        6611,
    );
    idx.on_add_on_thread(
        1031,
        1032,
        "Windows.UI.Xaml.Controls.Primitives.Popup".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1032,
        1033,
        "Windows.UI.Xaml.Controls.Grid".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1033,
        1034,
        "Windows.UI.Xaml.Controls.ListView".into(),
        "List".into(),
        6611,
    );
    assert_eq!(idx.classify_target(1031), None);
    assert_eq!(idx.classify_target(1033), None);

    // 4. Coexisting MenuFlyoutPresenter on the same thread remains unaffected
    idx.on_add_on_thread(
        0,
        1040,
        "Windows.UI.Xaml.Controls.MenuFlyoutPresenter".into(),
        "".into(),
        6611,
    );
    idx.on_add_on_thread(
        1040,
        1041,
        "Windows.UI.Xaml.Controls.Border".into(),
        "".into(),
        6611,
    );
    assert_eq!(idx.classify_target(1041), Some(TargetKind::MenuFlyoutSurface));
}
