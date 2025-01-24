function adjustViewBox(objectId) {
    const object = document.getElementById(objectId);
    object.addEventListener('load', () => {
        const svgDocument = object.contentDocument;
        const svgElement = svgDocument?.querySelector('svg');
        if (svgElement) {
            // Remove explicit width and height attributes to allow scaling
            svgElement.removeAttribute('width');
            svgElement.removeAttribute('height');
            // Ensure the aspect ratio is preserved during scaling
            svgElement.setAttribute('preserveAspectRatio', 'xMidYMid meet');

            // Get the current viewBox or calculate a new one based on the bounding box
            const bbox = svgElement.getBBox();
            svgElement.setAttribute(
                'viewBox',
                `${bbox.x} ${bbox.y} ${bbox.width} ${bbox.height}`
            );
        } else {
            console.warn(`SVG element not found in object with ID: ${objectId}`);
        }
    });
}

adjustViewBox('svg1');
adjustViewBox('svg2');
