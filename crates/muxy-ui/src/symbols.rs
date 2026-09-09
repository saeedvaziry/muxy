#[derive(Debug)]
pub struct Symbol {
    pub name: &'static str,
    pub symbol: &'static str,
}

pub const SYMBOLS: [Symbol; 192] = [
    Symbol {
        name: "Folder",
        symbol: "folder",
    },
    Symbol {
        name: "Folder Fill",
        symbol: "folder.fill",
    },
    Symbol {
        name: "Terminal",
        symbol: "terminal",
    },
    Symbol {
        name: "Terminal Fill",
        symbol: "terminal.fill",
    },
    Symbol {
        name: "Code",
        symbol: "chevron.left.forwardslash.chevron.right",
    },
    Symbol {
        name: "Document",
        symbol: "doc",
    },
    Symbol {
        name: "Document Fill",
        symbol: "doc.fill",
    },
    Symbol {
        name: "Document Text",
        symbol: "doc.text",
    },
    Symbol {
        name: "Document Text Fill",
        symbol: "doc.text.fill",
    },
    Symbol {
        name: "Document on Clipboard",
        symbol: "doc.on.clipboard",
    },
    Symbol {
        name: "List",
        symbol: "list.bullet",
    },
    Symbol {
        name: "List Rectangle",
        symbol: "list.bullet.rectangle",
    },
    Symbol {
        name: "Pencil",
        symbol: "pencil",
    },
    Symbol {
        name: "Pencil and Ruler",
        symbol: "pencil.and.ruler",
    },
    Symbol {
        name: "Ruler",
        symbol: "ruler",
    },
    Symbol {
        name: "Swatchpalette",
        symbol: "swatchpalette.fill",
    },
    Symbol {
        name: "Paintpalette",
        symbol: "paintpalette.fill",
    },
    Symbol {
        name: "Sparkle",
        symbol: "sparkle",
    },
    Symbol {
        name: "Star",
        symbol: "star",
    },
    Symbol {
        name: "Star Fill",
        symbol: "star.fill",
    },
    Symbol {
        name: "Gear",
        symbol: "gearshape",
    },
    Symbol {
        name: "Gear Fill",
        symbol: "gearshape.fill",
    },
    Symbol {
        name: "Wrench",
        symbol: "wrench",
    },
    Symbol {
        name: "Wrench Fill",
        symbol: "wrench.fill",
    },
    Symbol {
        name: "Hammer",
        symbol: "hammer",
    },
    Symbol {
        name: "Hammer Fill",
        symbol: "hammer.fill",
    },
    Symbol {
        name: "Screwdriver",
        symbol: "screwdriver",
    },
    Symbol {
        name: "Screwdriver Fill",
        symbol: "screwdriver.fill",
    },
    Symbol {
        name: "Wand and Stars",
        symbol: "wand.and.stars",
    },
    Symbol {
        name: "Globe",
        symbol: "globe",
    },
    Symbol {
        name: "Globe Americas",
        symbol: "globe.americas",
    },
    Symbol {
        name: "Network",
        symbol: "network",
    },
    Symbol {
        name: "Server Rack",
        symbol: "server.rack",
    },
    Symbol {
        name: "External Drive",
        symbol: "externaldrive",
    },
    Symbol {
        name: "External Drive Fill",
        symbol: "externaldrive.fill",
    },
    Symbol {
        name: "Optical Disc",
        symbol: "opticaldisc",
    },
    Symbol {
        name: "Optical Disc Fill",
        symbol: "opticaldisc.fill",
    },
    Symbol {
        name: "Cloud",
        symbol: "cloud",
    },
    Symbol {
        name: "Cloud Fill",
        symbol: "cloud.fill",
    },
    Symbol {
        name: "Square Grid 2x2",
        symbol: "square.grid.2x2",
    },
    Symbol {
        name: "Square Grid 2x2 Fill",
        symbol: "square.grid.2x2.fill",
    },
    Symbol {
        name: "Square Grid 3x2",
        symbol: "square.grid.3x2",
    },
    Symbol {
        name: "Square Grid 3x2 Fill",
        symbol: "square.grid.3x2.fill",
    },
    Symbol {
        name: "Rectangle Grid 1x2",
        symbol: "rectangle.grid.1x2",
    },
    Symbol {
        name: "Rectangle Grid 1x2 Fill",
        symbol: "rectangle.grid.1x2.fill",
    },
    Symbol {
        name: "Rectangle 3 Group",
        symbol: "rectangle.3.group",
    },
    Symbol {
        name: "Rectangle 3 Group Fill",
        symbol: "rectangle.3.group.fill",
    },
    Symbol {
        name: "Square Stack",
        symbol: "square.stack",
    },
    Symbol {
        name: "Square Stack Fill",
        symbol: "square.stack.fill",
    },
    Symbol {
        name: "Circle",
        symbol: "circle",
    },
    Symbol {
        name: "Circle Fill",
        symbol: "circle.fill",
    },
    Symbol {
        name: "Diamond",
        symbol: "diamond",
    },
    Symbol {
        name: "Diamond Fill",
        symbol: "diamond.fill",
    },
    Symbol {
        name: "Hexagon",
        symbol: "hexagon",
    },
    Symbol {
        name: "Hexagon Fill",
        symbol: "hexagon.fill",
    },
    Symbol {
        name: "App",
        symbol: "app",
    },
    Symbol {
        name: "App Fill",
        symbol: "app.fill",
    },
    Symbol {
        name: "App Badge",
        symbol: "app.badge",
    },
    Symbol {
        name: "App Badge Fill",
        symbol: "app.badge.fill",
    },
    Symbol {
        name: "App Window",
        symbol: "macwindow",
    },
    Symbol {
        name: "Play",
        symbol: "play",
    },
    Symbol {
        name: "Play Fill",
        symbol: "play.fill",
    },
    Symbol {
        name: "Play Rectangle",
        symbol: "play.rectangle",
    },
    Symbol {
        name: "Play Rectangle Fill",
        symbol: "play.rectangle.fill",
    },
    Symbol {
        name: "Music Note",
        symbol: "music.note",
    },
    Symbol {
        name: "Music Note List",
        symbol: "music.note.list",
    },
    Symbol {
        name: "Film",
        symbol: "film",
    },
    Symbol {
        name: "Film Fill",
        symbol: "film.fill",
    },
    Symbol {
        name: "Photo",
        symbol: "photo",
    },
    Symbol {
        name: "Photo Fill",
        symbol: "photo.fill",
    },
    Symbol {
        name: "Video",
        symbol: "video",
    },
    Symbol {
        name: "Video Fill",
        symbol: "video.fill",
    },
    Symbol {
        name: "Message",
        symbol: "message",
    },
    Symbol {
        name: "Message Fill",
        symbol: "message.fill",
    },
    Symbol {
        name: "Bubble Left",
        symbol: "bubble.left",
    },
    Symbol {
        name: "Bubble Left Fill",
        symbol: "bubble.left.fill",
    },
    Symbol {
        name: "Envelope",
        symbol: "envelope",
    },
    Symbol {
        name: "Envelope Fill",
        symbol: "envelope.fill",
    },
    Symbol {
        name: "Calendar",
        symbol: "calendar",
    },
    Symbol {
        name: "Clock",
        symbol: "clock",
    },
    Symbol {
        name: "Clock Fill",
        symbol: "clock.fill",
    },
    Symbol {
        name: "Timer",
        symbol: "timer",
    },
    Symbol {
        name: "Bell",
        symbol: "bell",
    },
    Symbol {
        name: "Bell Fill",
        symbol: "bell.fill",
    },
    Symbol {
        name: "Bell Badge",
        symbol: "bell.badge",
    },
    Symbol {
        name: "Bell Badge Fill",
        symbol: "bell.badge.fill",
    },
    Symbol {
        name: "Bookmark",
        symbol: "bookmark",
    },
    Symbol {
        name: "Bookmark Fill",
        symbol: "bookmark.fill",
    },
    Symbol {
        name: "Flag",
        symbol: "flag",
    },
    Symbol {
        name: "Flag Fill",
        symbol: "flag.fill",
    },
    Symbol {
        name: "Tag",
        symbol: "tag",
    },
    Symbol {
        name: "Tag Fill",
        symbol: "tag.fill",
    },
    Symbol {
        name: "Book",
        symbol: "book",
    },
    Symbol {
        name: "Book Fill",
        symbol: "book.fill",
    },
    Symbol {
        name: "Books Vertical",
        symbol: "books.vertical",
    },
    Symbol {
        name: "Books Vertical Fill",
        symbol: "books.vertical.fill",
    },
    Symbol {
        name: "Magnifying Glass",
        symbol: "magnifyingglass",
    },
    Symbol {
        name: "Lightbulb",
        symbol: "lightbulb",
    },
    Symbol {
        name: "Lightbulb Fill",
        symbol: "lightbulb.fill",
    },
    Symbol {
        name: "Cube",
        symbol: "cube",
    },
    Symbol {
        name: "Cube Fill",
        symbol: "cube.fill",
    },
    Symbol {
        name: "Cylinder",
        symbol: "cylinder",
    },
    Symbol {
        name: "Cylinder Fill",
        symbol: "cylinder.fill",
    },
    Symbol {
        name: "Capsule",
        symbol: "capsule",
    },
    Symbol {
        name: "Capsule Fill",
        symbol: "capsule.fill",
    },
    Symbol {
        name: "Pyramid",
        symbol: "pyramid",
    },
    Symbol {
        name: "Pyramid Fill",
        symbol: "pyramid.fill",
    },
    Symbol {
        name: "Cone",
        symbol: "cone",
    },
    Symbol {
        name: "Cone Fill",
        symbol: "cone.fill",
    },
    Symbol {
        name: "Key",
        symbol: "key",
    },
    Symbol {
        name: "Key Fill",
        symbol: "key.fill",
    },
    Symbol {
        name: "Lock",
        symbol: "lock",
    },
    Symbol {
        name: "Lock Fill",
        symbol: "lock.fill",
    },
    Symbol {
        name: "Lock Shield",
        symbol: "lock.shield",
    },
    Symbol {
        name: "Lock Shield Fill",
        symbol: "lock.shield.fill",
    },
    Symbol {
        name: "Shield",
        symbol: "shield",
    },
    Symbol {
        name: "Shield Fill",
        symbol: "shield.fill",
    },
    Symbol {
        name: "Bolt",
        symbol: "bolt",
    },
    Symbol {
        name: "Bolt Fill",
        symbol: "bolt.fill",
    },
    Symbol {
        name: "Bolt Shield",
        symbol: "bolt.shield",
    },
    Symbol {
        name: "Bolt Shield Fill",
        symbol: "bolt.shield.fill",
    },
    Symbol {
        name: "Waveform",
        symbol: "waveform",
    },
    Symbol {
        name: "Waveform Path",
        symbol: "waveform.path",
    },
    Symbol {
        name: "Chart Pie",
        symbol: "chart.pie",
    },
    Symbol {
        name: "Chart Pie Fill",
        symbol: "chart.pie.fill",
    },
    Symbol {
        name: "Chart Bar",
        symbol: "chart.bar",
    },
    Symbol {
        name: "Chart Bar Fill",
        symbol: "chart.bar.fill",
    },
    Symbol {
        name: "Chart Line",
        symbol: "chart.line.uptrend.xyaxis",
    },
    Symbol {
        name: "Gauge",
        symbol: "gauge",
    },
    Symbol {
        name: "Speedometer",
        symbol: "speedometer",
    },
    Symbol {
        name: "Viewfinder",
        symbol: "viewfinder",
    },
    Symbol {
        name: "Viewfinder Circle",
        symbol: "viewfinder.circle",
    },
    Symbol {
        name: "Camera",
        symbol: "camera",
    },
    Symbol {
        name: "Camera Fill",
        symbol: "camera.fill",
    },
    Symbol {
        name: "Eye",
        symbol: "eye",
    },
    Symbol {
        name: "Eye Fill",
        symbol: "eye.fill",
    },
    Symbol {
        name: "Eye Slash",
        symbol: "eye.slash",
    },
    Symbol {
        name: "Eye Slash Fill",
        symbol: "eye.slash.fill",
    },
    Symbol {
        name: "Person",
        symbol: "person",
    },
    Symbol {
        name: "Person Fill",
        symbol: "person.fill",
    },
    Symbol {
        name: "Person Circle",
        symbol: "person.circle",
    },
    Symbol {
        name: "Person Circle Fill",
        symbol: "person.circle.fill",
    },
    Symbol {
        name: "Person 2",
        symbol: "person.2",
    },
    Symbol {
        name: "Person 2 Fill",
        symbol: "person.2.fill",
    },
    Symbol {
        name: "Person 3",
        symbol: "person.3",
    },
    Symbol {
        name: "Person 3 Fill",
        symbol: "person.3.fill",
    },
    Symbol {
        name: "Backpack",
        symbol: "backpack",
    },
    Symbol {
        name: "Backpack Fill",
        symbol: "backpack.fill",
    },
    Symbol {
        name: "House",
        symbol: "house",
    },
    Symbol {
        name: "House Fill",
        symbol: "house.fill",
    },
    Symbol {
        name: "Building",
        symbol: "building",
    },
    Symbol {
        name: "Building Fill",
        symbol: "building.fill",
    },
    Symbol {
        name: "Building Columns",
        symbol: "building.columns",
    },
    Symbol {
        name: "Building Columns Fill",
        symbol: "building.columns.fill",
    },
    Symbol {
        name: "Cart",
        symbol: "cart",
    },
    Symbol {
        name: "Cart Fill",
        symbol: "cart.fill",
    },
    Symbol {
        name: "Bag",
        symbol: "bag",
    },
    Symbol {
        name: "Bag Fill",
        symbol: "bag.fill",
    },
    Symbol {
        name: "Creditcard",
        symbol: "creditcard",
    },
    Symbol {
        name: "Creditcard Fill",
        symbol: "creditcard.fill",
    },
    Symbol {
        name: "Link",
        symbol: "link",
    },
    Symbol {
        name: "Link Circle",
        symbol: "link.circle",
    },
    Symbol {
        name: "Link Circle Fill",
        symbol: "link.circle.fill",
    },
    Symbol {
        name: "Square and Arrow",
        symbol: "square.and.arrow.up",
    },
    Symbol {
        name: "Square and Arrow Fill",
        symbol: "square.and.arrow.up.fill",
    },
    Symbol {
        name: "Square on Square",
        symbol: "square.on.square",
    },
    Symbol {
        name: "Square on Square Fill",
        symbol: "square.on.square.fill",
    },
    Symbol {
        name: "Arrowshape",
        symbol: "arrowshape.turn.up.forward",
    },
    Symbol {
        name: "Arrowshape Fill",
        symbol: "arrowshape.turn.up.forward.fill",
    },
    Symbol {
        name: "Arrow Up",
        symbol: "arrow.up",
    },
    Symbol {
        name: "Arrow Down",
        symbol: "arrow.down",
    },
    Symbol {
        name: "Arrow Left",
        symbol: "arrow.left",
    },
    Symbol {
        name: "Arrow Right",
        symbol: "arrow.right",
    },
    Symbol {
        name: "Arrow Up Circle",
        symbol: "arrow.up.circle",
    },
    Symbol {
        name: "Arrow Up Circle Fill",
        symbol: "arrow.up.circle.fill",
    },
    Symbol {
        name: "Arrow Down Circle",
        symbol: "arrow.down.circle",
    },
    Symbol {
        name: "Arrow Down Circle Fill",
        symbol: "arrow.down.circle.fill",
    },
    Symbol {
        name: "Arrow Left Circle",
        symbol: "arrow.left.circle",
    },
    Symbol {
        name: "Arrow Left Circle Fill",
        symbol: "arrow.left.circle.fill",
    },
    Symbol {
        name: "Arrow Right Circle",
        symbol: "arrow.right.circle",
    },
    Symbol {
        name: "Arrow Right Circle Fill",
        symbol: "arrow.right.circle.fill",
    },
    Symbol {
        name: "Arrow Left Right",
        symbol: "arrow.left.and.right",
    },
    Symbol {
        name: "Arrow Up Down",
        symbol: "arrow.up.and.down",
    },
    Symbol {
        name: "Arrows",
        symbol: "arrow.triangle.branch",
    },
    Symbol {
        name: "Arrows Merge",
        symbol: "arrow.triangle.merge",
    },
    Symbol {
        name: "Arrows Swap",
        symbol: "arrow.triangle.swap",
    },
    Symbol {
        name: "Arrowshape Backward",
        symbol: "arrowshape.backward",
    },
    Symbol {
        name: "Arrowshape Forward",
        symbol: "arrowshape.forward",
    },
    Symbol {
        name: "Go Backward",
        symbol: "gobackward",
    },
    Symbol {
        name: "Go Forward",
        symbol: "goforward",
    },
    Symbol {
        name: "Forward",
        symbol: "forward",
    },
    Symbol {
        name: "Forward Fill",
        symbol: "forward.fill",
    },
];

pub fn matching(query: &str) -> Vec<&'static Symbol> {
    let query = query.trim().to_lowercase();
    SYMBOLS
        .iter()
        .filter(|symbol| {
            query.is_empty()
                || symbol.name.to_lowercase().contains(&query)
                || symbol.symbol.to_lowercase().contains(&query)
        })
        .collect()
}
